const test = require('node:test'), assert = require('node:assert/strict')
const { ed25519 } = require('@noble/curves/ed25519')
const { ingest } = require('../validation')
const MigrationManager = require('../db/migration-manager')
test('PostgreSQL migrations, atomic validation, duplicate races and conflicting nonce rollback', {skip:!process.env.TEST_DATABASE_URL}, async()=>{
 const pgp=require('pg-promise')(), db=pgp(process.env.TEST_DATABASE_URL)
 try {
  await db.task(c=>new MigrationManager(c,require('path').join(__dirname,'../db/migrations')).runMigrations())
  await db.none('TRUNCATE m3ter_payloads')
  const secret=new Uint8Array(32).fill(45),key=Buffer.from(ed25519.getPublicKey(secret)).toString('hex')
  function action(n,energy=1000){const b=Buffer.alloc(8);b.writeUInt32BE(n);b.writeUInt32BE(energy,4);return {m3ter_id:7,message:b.toString('hex')+Buffer.from(ed25519.sign(b,secret)).toString('hex')}}
  const chain={snapshot:async()=>({meter:async()=>({key,nonce:0})})}
  const results=await Promise.all([ingest(db,chain,[action(1)],'https'),ingest(db,chain,[action(1)],'streamr')])
  assert.equal(results.reduce((s,r)=>s+r.accepted,0),1)
  assert.equal((await db.one('SELECT count(*)::integer AS n FROM m3ter_payloads')).n,1)
  await assert.rejects(ingest(db,chain,[action(2),action(1,2000)],'https'))
  assert.equal((await db.one('SELECT count(*)::integer AS n FROM m3ter_payloads')).n,1)
  const invalid=action(2);invalid.message=invalid.message.slice(0,16)+'00'.repeat(64)
  await assert.rejects(ingest(db,chain,[action(3),invalid],'streamr'))
  assert.equal((await db.one('SELECT count(*)::integer AS n FROM m3ter_payloads')).n,1)
 } finally {pgp.end()}
})
