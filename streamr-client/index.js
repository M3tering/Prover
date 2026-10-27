'use strict'
require('dotenv').config({ quiet: true })
const express = require('express')
const pgp = require('pg-promise')()
const { Chain, InvalidPayload, ingest } = require('./validation')
const MigrationManager = require('./db/migration-manager')

async function main() {
  for (const key of ['DATABASE_URL', 'RPC_URL', 'ROLLUP_ADDRESS', 'CHAIN_ID']) {
    if (!process.env[key]) throw new Error(`Set ${key}`)
  }
  const db = pgp(process.env.DATABASE_URL)
  // A dedicated connection keeps the migration advisory lock on the same session.
  await db.task(async connection => new MigrationManager(connection, require('path').join(__dirname, 'db/migrations')).runMigrations())
  const identity = [String(process.env.CHAIN_ID), process.env.ROLLUP_ADDRESS.toLowerCase()]
  await db.none('INSERT INTO node_identity(singleton,chain_id,rollup) VALUES(true,$1,$2) ON CONFLICT DO NOTHING', identity)
  const bound = await db.one('SELECT chain_id,rollup FROM node_identity WHERE singleton')
  if (bound.chain_id !== identity[0] || bound.rollup !== identity[1]) throw new Error('Database belongs to another chain/Rollup; use a new volume')
  const chain = new Chain(process.env.RPC_URL, process.env.ROLLUP_ADDRESS, process.env.CHAIN_ID)
  await chain.snapshot()
  const app = express()
  app.disable('x-powered-by')
  app.use(express.json({ limit: '256kb', strict: true }))
  let active = false, subscription, streamr, server
  const state = { streamr: process.env.STREAM_ID ? 'connecting' : 'disabled' }
  async function accept(items, source) {
    if (active) { const e = new Error('ingestion busy; retry'); e.status = 429; throw e }
    active = true
    try { return await ingest(db, chain, items, source, Number(process.env.MAX_BATCH_ACTIONS || 1000)) }
    finally { active = false }
  }
  app.get('/health', async (_req, res) => {
    try { await db.one('SELECT 1 AS ok'); res.json({ status: 'ok', chainId: identity[0], rollup: identity[1], ...state }) }
    catch { res.status(503).json({ status: 'database unavailable' }) }
  })
  app.post('/batch-payloads', async (req, res) => {
    try { res.status(202).json(await accept(req.body, 'https')) }
    catch (error) {
      const status = error instanceof InvalidPayload ? 400 : error.status || 503
      res.status(status).json({ error: status === 503 ? 'validation unavailable; retry later' : error.message })
    }
  })
  app.use((error, _req, res, _next) => res.status(error.status === 413 ? 413 : 400).json({ error: 'invalid JSON or request too large' }))
  server = app.listen(Number(process.env.PORT || 3000), process.env.BIND_ADDRESS || '0.0.0.0', () => console.log('HTTP ingestion listening'))
  // The proving worker runs in a separate container without a public listener.
  if (process.env.STREAM_ID) {
    const { StreamrClient } = require('@streamr/sdk')
    streamr = new StreamrClient(process.env.STREAMR_PRIVATE_KEY ? { auth: { privateKey: process.env.STREAMR_PRIVATE_KEY } } : {})
    // HTTP and the scheduler remain available while Streamr reconnects.
    ;(async () => {
      while (!subscription) {
        try {
          subscription = await streamr.subscribe(process.env.STREAM_ID, async data => {
            try { const result = await accept(data, 'streamr'); console.log(`Streamr accepted ${result.accepted} actions`) }
            catch (error) { console.warn('Streamr batch not queued:', error instanceof InvalidPayload ? error.message : 'temporarily unavailable') }
          })
          state.streamr = 'subscribed'
          subscription.on('error', () => { state.streamr = 'error'; console.warn('Streamr subscription error') })
        } catch { state.streamr = 'retrying'; await new Promise(resolve => setTimeout(resolve, 30000)) }
      }
    })()
  }
  const stop = async () => {
    console.log('Stopping node')
    server.close()
    if (streamr) await streamr.destroy()
    pgp.end()
    process.exit(0)
  }
  process.on('SIGTERM', stop); process.on('SIGINT', stop)
}
main().catch(() => { console.error('Node startup failed; check configuration, database, chain and Rollup deployment'); process.exit(1) })
