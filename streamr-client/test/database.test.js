const test = require('node:test'), assert = require('node:assert/strict')
const { ed25519 } = require('@noble/curves/ed25519')
const { ingest } = require('../validation')
const MigrationManager = require('../db/migration-manager')
test('PostgreSQL keeps only canonical validated actions despite mixed input and concurrent retries', {skip:!process.env.TEST_DATABASE_URL}, async()=>{
 const pgp=require('pg-promise')(), db=pgp(process.env.TEST_DATABASE_URL)
 try {
  await db.task(c=>new MigrationManager(c,require('path').join(__dirname,'../db/migrations')).runMigrations())
  await db.none('TRUNCATE m3ter_payloads')
  const secret=new Uint8Array(32).fill(45),key=Buffer.from(ed25519.getPublicKey(secret)).toString('hex')
  function action(n,energy=1000){const b=Buffer.alloc(8);b.writeUInt32BE(n);b.writeUInt32BE(energy,4);return {m3ter_id:7,message:b.toString('hex')+Buffer.from(ed25519.sign(b,secret)).toString('hex')}}
  const chain={snapshot:async()=>({meter:async()=>({key,nonce:0})})}
  const results=await Promise.all([ingest(db,chain,[action(1)],'https'),ingest(db,chain,[action(1)],'streamr')])
  assert.equal(results.reduce((s,r)=>s+r.accepted,0),1)
  assert.equal(results.reduce((s,r)=>s+r.duplicates,0),1)
  assert.equal((await db.one('SELECT count(*)::integer AS n FROM m3ter_payloads')).n,1)
  assert.deepEqual(await ingest(db,chain,[action(2),action(1,2000)],'https'),{accepted:1,duplicates:0,discarded:1})
  const invalid=action(3);invalid.message=invalid.message.slice(0,16)+'00'.repeat(64)
  const tailed={...action(3),message:'0x'+action(3).message+'extra bytes are ignored'}
  assert.deepEqual(await ingest(db,chain,[invalid,tailed,action(3),action(3,2000),null,{m3ter_id:7,message:'00'}],'streamr'),{accepted:1,duplicates:1,discarded:4})
  const rows=await db.any('SELECT nonce,energy,message,signature FROM m3ter_payloads ORDER BY nonce')
  assert.equal(rows.length,3)
  for(const row of rows){
   assert.equal(row.message,action(row.nonce).message.slice(0,16))
   assert.equal(row.signature,action(row.nonce).message.slice(16))
   assert.equal(Number(row.energy),1000)
  }
  const consumed={snapshot:async()=>({meter:async()=>({key,nonce:3})})}
  assert.deepEqual(await ingest(db,consumed,[action(1),action(4)],'https'),{accepted:1,duplicates:0,discarded:1})
  const conflict=await Promise.all([ingest(db,chain,[action(5,1000)],'https'),ingest(db,chain,[action(5,2000)],'https')])
  assert.equal(conflict.reduce((s,r)=>s+r.accepted,0),1)
  assert.equal(conflict.reduce((s,r)=>s+r.discarded,0),1)
  // A DB outage must roll back the whole transaction and propagate for retry.
  const failingDb={tx:callback=>db.tx(tx=>callback({one:tx.one.bind(tx),oneOrNone:async(sql,args)=>{
   if(args[1]===7)throw Error('simulated database failure')
   return tx.oneOrNone(sql,args)
  }}))}
  await assert.rejects(ingest(failingDb,chain,[action(6),action(7)],'https'),/simulated database failure/)
  assert.equal((await db.one('SELECT count(*)::integer AS n FROM m3ter_payloads')).n,5)
 } finally {pgp.end()}
})
