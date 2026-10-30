'use strict'
const express = require('express')

function createApp(accept, health) {
  const app = express()
  app.disable('x-powered-by')
  app.use(express.json({ limit: '256kb', strict: false }))
  app.get('/health', async (_req, res) => {
    try { res.json(await health()) }
    catch { res.status(503).json({ status: 'database unavailable' }) }
  })
  app.post('/batch-payloads', async (req, res) => {
    try { res.status(202).json(await accept(req.body, 'https')) }
    catch (error) {
      const status = [413, 429].includes(error.status) ? error.status : 503
      res.status(status).json({ error: status === 503 ? 'validation unavailable; retry later' : error.message })
    }
  })
  app.use((error, _req, res, _next) => {
    const status = error.status === 413 ? 413 : 400
    res.status(status).json({ error: status === 413 ? 'request too large' : 'invalid JSON' })
  })
  return app
}
module.exports = { createApp }
