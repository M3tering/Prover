'use strict'
const fs = require('node:fs')
const path = require('node:path')
const { spawn } = require('node:child_process')
const { authenticate } = require('./validation')

// Refresh eligibility against one confirmed chain snapshot. A missing nonce waits;
// it cannot stall unrelated meters. Recheck invalidated signatures after rotation.
async function prepare(db, chain, limit = 10000) {
  const snapshot = await chain.snapshot('finalized')
  const rows = await db.any(`SELECT * FROM m3ter_payloads WHERE NOT is_verified
    ORDER BY checked_at NULLS FIRST,m3ter_id,nonce LIMIT $1`, [limit])
  rows.sort((a,b) => Number(a.m3ter_id) - Number(b.m3ter_id) || Number(a.nonce) - Number(b.nonce))
  const states = new Map()
  for (const row of rows) {
    const id = Number(row.m3ter_id)
    if (!states.has(id)) states.set(id, { ...await snapshot.meter(id), energy: 0n })
    const state = states.get(id)
    let status = 'waiting', reason = 'nonce gap', consumed = false
    if (Number(row.nonce) <= state.nonce && !state.advanced) { consumed = true; status = 'consumed'; reason = null }
    else {
      try {
        authenticate({ ...row, m3ter_id: id }, state.key)
        if (Number(row.nonce) === state.nonce + 1) {
          const energy = state.energy + BigInt(row.energy)
          if (state.account + energy / 1000n > (1n << 40n) - 1n) { status = 'waiting'; reason = 'account capacity exhausted' }
          else { status = 'ready'; reason = null; state.nonce++; state.energy = energy; state.advanced = true }
        }
      } catch { status = 'waiting'; reason = 'signature does not match current key' }
    }
    await db.none('UPDATE m3ter_payloads SET queue_status=$2,reason=$3,is_verified=$4,checked_at=now() WHERE id=$1', [row.id, status, reason, consumed])
  }
  return rows.length
}
function unresolved(root) {
  if (!fs.existsSync(root)) return []
  return fs.readdirSync(root).filter(name => {
    const dir = path.join(root, name)
    return fs.existsSync(path.join(dir, 'paid-attempt.json')) && !fs.existsSync(path.join(dir, 'complete.json'))
  })
}
function startWorker(db, chain, state) {
  const root = process.env.BATCH_DIR || '/data/batches'
  fs.mkdirSync(root, { recursive: true })
  state = new Proxy(state, { set(object, name, value) {
    object[name] = value
    const file = path.join(path.dirname(root), 'status.json')
    fs.writeFileSync(file + '.tmp', JSON.stringify({ ...object, updatedAt: new Date().toISOString() }))
    fs.renameSync(file + '.tmp', file)
    return true
  } })
  let stopping = false, child, timer, activeTask = Promise.resolve()
  const interval = Number(process.env.PROVE_INTERVAL_SECONDS || 43200)
  if (!Number.isSafeInteger(interval) || interval < 30) throw new Error('PROVE_INTERVAL_SECONDS must be >=30')
  async function runHost(out, command, maxActions = process.env.MAX_ACTIONS || 10000) {
    const logfile = fs.openSync(path.join(out, 'host.log'), 'a', 0o600)
    const exit = await new Promise((resolve, reject) => {
      child = spawn(process.env.PROVER_BINARY || '/usr/local/bin/m3tering-prover', [command], {
        env: { ...process.env, OUTPUT_DIR: out, MAX_ACTIONS: String(maxActions) }, stdio: ['ignore', logfile, logfile]
      })
      child.on('error', reject); child.on('exit', (code, signal) => {
        if (signal === 'SIGKILL' && !fs.existsSync(path.join(out, 'paid-attempt.json'))) {
          fs.writeFileSync(path.join(out, 'reduce-batch.json'), JSON.stringify({reason: 'local execution killed before purchase'}))
        }
        resolve(code === null ? 1 : code)
      })
    }).finally(() => { fs.closeSync(logfile); child = null })
    state.worker = exit === 0 ? 'idle' : 'batch failed; inspect host.log'
    console.log(`Batch ${command} finished with exit ${exit}: ${path.basename(out)}`)
    return exit
  }
  async function tick() {
    if (stopping) return
    try {
      const blocked = unresolved(root)
      if (blocked.length) {
        if (process.env.PROVING_ENABLED !== 'true') { state.worker = 'recovery paused by configuration'; return }
        const out = path.join(root, blocked[0])
        if (['request.json','proof.bin','transaction.json'].some(name => fs.existsSync(path.join(out,name)))) {
          state.worker = 'recovering paid batch'
          await runHost(out,'recover')
        } else { state.worker = 'paused: ambiguous paid request'; console.warn('Inspect SP1 request history before retrying:', blocked[0]) }
        return
      }
      state.worker = 'checking queue'
      await prepare(db, chain, Number(process.env.MAX_ACTIONS || 10000))
      const ready = await db.one("SELECT count(*)::integer AS count FROM m3ter_payloads WHERE NOT is_verified AND queue_status='ready'")
      if (!ready.count) { state.worker = 'idle'; return }
      if (process.env.PROVING_ENABLED !== 'true') { state.worker = 'paused by configuration'; return }
      let maxActions = Number(process.env.MAX_ACTIONS || 10000)
      // Halve only after a local cycle/PGU admission failure, before any purchase.
      while (!stopping) {
        const out = path.join(root, new Date().toISOString().replace(/[:.]/g, '-') + '-' + require('crypto').randomBytes(4).toString('hex'))
        fs.mkdirSync(out)
        state.worker = 'proving'
        const exit = await runHost(out, 'run', maxActions)
        if (exit === 0 || maxActions <= 1 || fs.existsSync(path.join(out,'paid-attempt.json')) || !fs.existsSync(path.join(out,'reduce-batch.json'))) break
        maxActions = Math.max(1, Math.floor(maxActions / 2))
        console.log(`Reducing local execution batch to ${maxActions} actions before any proof purchase`)
      }
    } catch { state.worker = 'retrying after error'; console.warn('Worker check failed; retrying at next interval') }
    finally { if (!stopping) timer = setTimeout(() => { activeTask = tick() }, interval * 1000) }
  }
  // Always wait until startup initialization completes.
  timer = setTimeout(() => { activeTask = tick() }, 1000)
  return { async stop() { stopping = true; clearTimeout(timer); if (child) child.kill('SIGTERM'); await activeTask } }
}
module.exports = { prepare, unresolved, startWorker }
