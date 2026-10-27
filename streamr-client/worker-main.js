'use strict'
require('dotenv').config({ quiet: true })
const pgp = require('pg-promise')()
const { Chain } = require('./validation')
const { startWorker } = require('./worker')
async function main() {
  const db = pgp(process.env.DATABASE_URL)
  const identity = await db.one('SELECT chain_id,rollup FROM node_identity WHERE singleton')
  if (identity.chain_id !== process.env.CHAIN_ID || identity.rollup !== process.env.ROLLUP_ADDRESS.toLowerCase()) throw new Error('Database identity mismatch')
  const chain = new Chain(process.env.RPC_URL, process.env.ROLLUP_ADDRESS, process.env.CHAIN_ID)
  await chain.snapshot()
  const worker = startWorker(db, chain, {})
  for (const signal of ['SIGTERM','SIGINT']) process.on(signal, async () => { await worker.stop(); pgp.end(); process.exit(0) })
}
main().catch(()=>{console.error('Worker startup failed; check configuration, database and chain');process.exit(1)})
