'use strict'
const { ed25519 } = require('@noble/curves/ed25519')
const { Interface } = require('ethers')
const abi = new Interface([
  'function publicKey(uint64) view returns (bytes32)',
  'function words(uint256) view returns (uint256)',
  'function wordCount() view returns (uint256)'
])
class InvalidPayload extends Error {}
function hex(value, bytes) {
  if (typeof value !== 'string' || !new RegExp(`^(?:0x)?[a-fA-F0-9]{${bytes * 2}}$`).test(value)) {
    throw new InvalidPayload(`expected exactly ${bytes} bytes of hex`)
  }
  return value.replace(/^0x/, '').toLowerCase()
}
function decode(item) {
  if (!item || !Number.isSafeInteger(item.m3ter_id) || item.m3ter_id < 0 || item.m3ter_id > 17179869183) {
    throw new InvalidPayload('m3ter_id must be an integer in 0..17179869183')
  }
  const packed = hex(item.message, 72)
  const bytes = Buffer.from(packed, 'hex')
  const nonce = bytes.readUInt32BE(0)
  if (nonce < 1 || nonce > 16777215) throw new InvalidPayload('nonce must be in 1..16777215')
  return { m3ter_id: item.m3ter_id, nonce, energy: bytes.readUInt32BE(4),
    message: packed.slice(0, 16), signature: packed.slice(16) }
}
function authenticate(action, key) {
  const raw = hex(key, 32)
  try {
    if (/^0+$/.test(raw) || !ed25519.verify(action.signature, action.message, raw, { zip215: false })) {
      throw new Error('signature')
    }
  } catch { throw new InvalidPayload(`invalid signature or unregistered key for meter ${action.m3ter_id}`) }
}
class Chain {
  constructor(url, rollup, chainId) { this.url = url; this.rollup = rollup; this.chainId = chainId }
  async rpc(method, params) {
    let response
    try {
      response = await fetch(this.url, { method: 'POST', headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }), signal: AbortSignal.timeout(20000) })
    } catch { throw new Error('Ethereum RPC unavailable') }
    if (!response.ok) throw new Error('Ethereum RPC HTTP failure')
    const data = await response.json()
    if (data.error || data.result == null) throw new Error(`Ethereum RPC ${method} failed`)
    return data.result
  }
  async view(name, args, tag) {
    const result = await this.rpc('eth_call', [{ to: this.rollup, data: abi.encodeFunctionData(name, args) }, tag])
    return abi.decodeFunctionResult(name, result)[0]
  }
  async snapshot(finality = 'latest') {
    if (BigInt(await this.rpc('eth_chainId', [])) !== BigInt(this.chainId)) throw new Error('wrong Ethereum chain')
    const block = await this.rpc('eth_getBlockByNumber', [finality, false])
    const tag = { blockHash: block.hash, requireCanonical: true }
    const count = await this.view('wordCount', [], tag)
    const words = new Map(), keys = new Map()
    return { block, meter: async id => {
      const slot = Math.floor(id / 4)
      if (!words.has(slot)) words.set(slot, BigInt(slot) < count ? await this.view('words', [slot], tag) : 0n)
      if (!keys.has(id)) keys.set(id, await this.view('publicKey', [id], tag))
      const pair = words.get(slot) >> BigInt(64 * (id % 4))
      return { key: keys.get(id), account: pair & ((1n << 40n) - 1n), nonce: Number((pair >> 40n) & 0xffffffn) }
    } }
  }
}
async function ingest(db, chain, items, source, max = 1000) {
  if (!Array.isArray(items) || !items.length || items.length > max) throw new InvalidPayload(`batch must contain 1..${max} actions`)
  // Validate the entire batch before inserting anything. Ignore unsigned metadata.
  const actions = items.map(decode), snapshot = await chain.snapshot()
  const seen = new Map()
  for (const action of actions) {
    const state = await snapshot.meter(action.m3ter_id)
    authenticate(action, state.key)
    if (action.nonce <= state.nonce) throw new InvalidPayload(`nonce already consumed for meter ${action.m3ter_id}`)
    const k = `${action.m3ter_id}:${action.nonce}`
    if (seen.has(k) && seen.get(k).message !== action.message) throw new InvalidPayload('conflicting nonce in batch')
    seen.set(k, action)
  }
  return db.tx(async tx => {
    let inserted = 0
    for (const action of seen.values()) {
      const row = await tx.oneOrNone(`INSERT INTO m3ter_payloads
        (m3ter_id,nonce,energy,message,signature,is_verified,source,queue_status)
        VALUES ($1,$2,$3,$4,$5,false,$6,'pending') ON CONFLICT (m3ter_id,nonce) DO NOTHING RETURNING id`,
      [action.m3ter_id, action.nonce, action.energy, action.message, action.signature, source])
      if (row) inserted++
      else {
        const prior = await tx.one('SELECT message,signature FROM m3ter_payloads WHERE m3ter_id=$1 AND nonce=$2', [action.m3ter_id, action.nonce])
        if (prior.message !== action.message || prior.signature !== action.signature) throw new InvalidPayload('conflicting action already queued for this nonce')
      }
    }
    return { accepted: inserted, duplicates: actions.length - inserted }
  })
}
module.exports = { decode, authenticate, Chain, InvalidPayload, ingest }
