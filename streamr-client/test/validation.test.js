const test = require('node:test')
const assert = require('node:assert/strict')
const { ed25519 } = require('@noble/curves/ed25519')
const { decode, authenticate, ingest } = require('../validation')
const { prepare, unresolved } = require('../worker')
const fs = require('node:fs'), os = require('node:os'), path = require('node:path')
const secret = new Uint8Array(32).fill(42), key = Buffer.from(ed25519.getPublicKey(secret)).toString('hex')
function signed(id, nonce, energy = 600) {
  const message = Buffer.alloc(8); message.writeUInt32BE(nonce); message.writeUInt32BE(energy, 4)
  return { m3ter_id: id, message: message.toString('hex') + Buffer.from(ed25519.sign(message, secret)).toString('hex') }
}
test('wire prefix, signed fields and capacity', () => {
  const action = decode({ ...signed(22, 1), nonce: 999, energy: 999 })
  assert.equal(action.nonce, 1); assert.equal(action.energy, 600); authenticate(action, key)
  for (const value of [signed(-1,1), signed(17179869184,1), signed(1,0), signed(1,16777216), {m3ter_id:1,message:signed(1,1).message.slice(0,-2)}]) assert.throws(()=>decode(value))
  for (const suffix of ['00'.repeat(500), 'garbage', 'f']) {
    const tailed = decode({m3ter_id:22,message:'0X'+signed(22,1).message+suffix})
    assert.deepEqual(tailed, action); authenticate(tailed,key)
  }
  assert.throws(()=>decode({m3ter_id:22,message:'zz'+signed(22,1).message}))
  const tampered = { ...action, message: '00000001000007d0' }
  assert.throws(()=>authenticate(tampered,key)); assert.throws(()=>authenticate(action,'00'.repeat(32)))
  assert.throws(()=>authenticate({...action,signature:'00'.repeat(64)},key))
})
test('invalid entries are discarded without DB writes; RPC failures remain retryable', async () => {
  let writes = 0
  const db = { tx: async () => { writes++ } }
  const chain = { snapshot: async () => ({meter: async () => ({key,nonce:0})}) }
  const bad = signed(23,1); bad.message = bad.message.slice(0,16) + '00'.repeat(64)
  assert.deepEqual(await ingest(db,chain,[bad,{message:'bad'},null,signed(1,0)],'https'),{accepted:0,duplicates:0,discarded:4})
  assert.deepEqual(await ingest(db,{snapshot:async()=>({meter:async()=>({key,nonce:1})})},[signed(22,1)],'https'),{accepted:0,duplicates:0,discarded:1})
  assert.deepEqual(await ingest(db,chain,[],'https'),{accepted:0,duplicates:0,discarded:0})
  const unavailable={snapshot:async()=>({meter:async id=>{if(id===23)throw Error('RPC unavailable');return {key,nonce:0}}})}
  await assert.rejects(ingest(db,unavailable,[signed(22,1),signed(23,1)],'https'),/RPC unavailable/)
  assert.equal(writes,0)
})
test('nonce gaps do not block other meters; consecutive actions aggregate before flooring', async () => {
  const rows = [signed(1,2),signed(2,1),signed(2,2)].map((x,i)=>({...decode(x),id:i+1}))
  const updates=[]
  await prepare({any:async()=>rows,none:async(_,args)=>updates.push(args)}, {snapshot:async()=>({meter:async()=>({key,nonce:0,account:0n})})})
  assert.deepEqual(updates.map(x=>x[1]), ['waiting','ready','ready'])
  assert.deepEqual(updates.map(x=>x[3]), [false,false,false])
})
test('paid-attempt marker blocks fresh proving across restart until completion', () => {
  const root=fs.mkdtempSync(path.join(os.tmpdir(),'prover-test-'));const dir=path.join(root,'batch');fs.mkdirSync(dir)
  assert.deepEqual(unresolved(root),[]);fs.writeFileSync(path.join(dir,'paid-attempt.json'),'{}')
  assert.deepEqual(unresolved(root),['batch']);fs.writeFileSync(path.join(dir,'complete.json'),'{}');assert.deepEqual(unresolved(root),[])
  fs.rmSync(root,{recursive:true})
})
module.exports={signed,key}
test('scheduler reduces only unpaid local-admission failures and stops once a paid marker exists', async () => {
  const { startWorker } = require('../worker')
  const root=fs.mkdtempSync(path.join(os.tmpdir(),'scheduler-test-'))
  const binary=path.join(root,'fake-host')
  fs.writeFileSync(binary,`#!/usr/bin/env node
const fs=require('fs'),path=require('path');const out=process.env.OUTPUT_DIR;
fs.writeFileSync(path.join(out,'actions'),process.env.MAX_ACTIONS);
fs.writeFileSync(path.join(out,Number(process.env.MAX_ACTIONS)>1?'reduce-batch.json':'paid-attempt.json'),'{}');process.exit(1);
`,{mode:0o700})
  const saved={...process.env}
  Object.assign(process.env,{BATCH_DIR:path.join(root,'batches'),PROVER_BINARY:binary,PROVING_ENABLED:'true',PROVE_INTERVAL_SECONDS:'30',MAX_ACTIONS:'4'})
  const worker=startWorker({any:async()=>[],one:async()=>({count:1})},{snapshot:async()=>({})},{})
  try {
    for(let i=0;i<100;i++){
      const dirs=fs.existsSync(path.join(root,'batches'))?fs.readdirSync(path.join(root,'batches')):[]
      if(dirs.some(d=>fs.existsSync(path.join(root,'batches',d,'paid-attempt.json'))))break
      await new Promise(r=>setTimeout(r,100))
    }
    const values=fs.readdirSync(path.join(root,'batches')).map(d=>Number(fs.readFileSync(path.join(root,'batches',d,'actions'),'utf8'))).sort((a,b)=>b-a)
    assert.deepEqual(values,[4,2,1])
  } finally {await worker.stop();for(const k of Object.keys(process.env))if(!(k in saved))delete process.env[k];Object.assign(process.env,saved);fs.rmSync(root,{recursive:true})}
})
