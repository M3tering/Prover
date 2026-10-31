const test = require('node:test'), assert = require('node:assert/strict')
const { ed25519 } = require('@noble/curves/ed25519')
const { once } = require('node:events')
const { createApp } = require('../http')
const { ingest } = require('../validation')

test('HTTP acknowledges and filters individual payloads while retaining transport and outage errors', async () => {
  const secret = new Uint8Array(32).fill(46), key = Buffer.from(ed25519.getPublicKey(secret)).toString('hex')
  function action(n) {
    const bytes=Buffer.alloc(8);bytes.writeUInt32BE(n);bytes.writeUInt32BE(1000,4)
    return {m3ter_id:7,message:bytes.toString('hex')+Buffer.from(ed25519.sign(bytes,secret)).toString('hex')}
  }
  const stored=new Map()
  let unavailable=false,busy=false,dbUnavailable=false
  const db={tx:async cb=>{
    if(dbUnavailable)throw Error('private database error')
    return cb({
      oneOrNone:async(_sql,args)=>{
        const id=args[0]+':'+args[1]
        if(stored.has(id))return null
        stored.set(id,{message:args[3],signature:args[4]});return {id}
      },
      one:async(_sql,args)=>stored.get(args[0]+':'+args[1])
    })
  }}
  const chain={snapshot:async()=>{
    if(unavailable)throw Error('private RPC error')
    return {meter:async()=>({key,nonce:1})}
  }}
  const app=createApp((body,source)=>{
    if(busy){const e=Error('ingestion busy; retry');e.status=429;throw e}
    return ingest(db,chain,body,source)
  },async()=>({status:'ok'}))
  const server=app.listen(0,'127.0.0.1');await once(server,'listening')
  async function post(body,raw=false) {
    const response=await fetch(`http://127.0.0.1:${server.address().port}/batch-payloads`,{
      method:'POST',headers:{'content-type':'application/json'},body:raw?body:JSON.stringify(body)
    })
    return {status:response.status,body:await response.json()}
  }
  try {
    const valid=action(2),tailed={...valid,message:valid.message+'ff'.repeat(500)}
    const invalid={...action(3),message:action(3).message.slice(0,16)+'00'.repeat(64)+action(3).message}
    assert.deepEqual(await post([tailed,invalid,action(1)]),{status:202,body:{accepted:1,duplicates:0,discarded:2}})
    assert.deepEqual(stored.get('7:2'),{message:valid.message.slice(0,16),signature:valid.message.slice(16)})
    assert.deepEqual(await post(valid),{status:202,body:{accepted:0,duplicates:1,discarded:0}})
    assert.deepEqual(await post([]),{status:202,body:{accepted:0,duplicates:0,discarded:0}})
    assert.deepEqual(await post(null),{status:202,body:{accepted:0,duplicates:0,discarded:1}})
    assert.equal((await post('{',true)).status,400)
    assert.equal((await post({message:'x'.repeat(256*1024)})).status,413)
    assert.equal((await post(Array(1001).fill(null))).status,413)
    unavailable=true
    assert.deepEqual(await post(action(3)),{status:503,body:{error:'validation unavailable; retry later'}})
    unavailable=false;dbUnavailable=true
    assert.equal((await post(action(3))).status,503)
    dbUnavailable=false;busy=true
    assert.equal((await post(action(3))).status,429)
    assert.equal(stored.size,1)
  } finally {server.closeAllConnections();await new Promise(resolve=>server.close(resolve))}
})
