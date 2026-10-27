'use strict'
const fs = require('node:fs'), crypto = require('node:crypto'), readline = require('node:readline')
const { Writable } = require('node:stream')
let hidden = false
const output = new Writable({write(chunk,_encoding,done){if(!hidden)process.stdout.write(chunk);done()}})
const rl = readline.createInterface({input:process.stdin,output,terminal:true})
const ask = (label,secret=false) => new Promise(resolve=>{process.stdout.write(label+': ');hidden=secret;rl.question('',answer=>{hidden=false;if(secret)process.stdout.write('\n');resolve(answer.trim())})})
async function main(){
  if(fs.existsSync('.env'))throw new Error('.env already exists; edit it directly to preserve credentials')
  const env={COMPOSE_PROJECT_NAME:'m3tering',POSTGRES_PASSWORD:crypto.randomBytes(32).toString('hex'),CHAIN_ID:'11155111',HTTP_PORT:'19000'}
  env.RPC_URL=await ask('Sepolia RPC URL');if(!/^https?:\/\//.test(env.RPC_URL))throw new Error('Invalid RPC URL')
  env.ROLLUP_ADDRESS=await ask('Rollup address');if(!/^0x[0-9a-fA-F]{40}$/.test(env.ROLLUP_ADDRESS))throw new Error('Invalid Rollup address')
  env.PRIVATE_KEY=await ask('Ethereum private key (hidden)',true);if(!/^(0x)?[0-9a-fA-F]{64}$/.test(env.PRIVATE_KEY))throw new Error('Invalid Ethereum key')
  env.SP1_PRIVATE_KEY=await ask('SP1 key (hidden; Enter to use the Ethereum key)',true)||env.PRIVATE_KEY
  if(!/^(0x)?[0-9a-fA-F]{64}$/.test(env.SP1_PRIVATE_KEY))throw new Error('Invalid SP1 key')
  env.STREAM_ID=await ask('Streamr stream ID (Enter to disable Streamr)')
  env.PROVE_INTERVAL_SECONDS=await ask('Proof interval in seconds [3600]')||'3600'
  if(!/^\d+$/.test(env.PROVE_INTERVAL_SECONDS)||Number(env.PROVE_INTERVAL_SECONDS)<30)throw new Error('Interval must be at least 30 seconds')
  env.MAX_PRICE_PER_PGU='auto';env.PROVING_ENABLED=(await ask('Enable paid scheduled proving? [yes/no]')).toLowerCase()==='yes'?'true':'false'
  env.MAX_PROOF_COST_WEI=await ask('Optional maximum PROVE wei per proof (Enter for live network quote)')
  if(env.MAX_PROOF_COST_WEI&&!/^\d+$/.test(env.MAX_PROOF_COST_WEI))throw new Error('Cost must be a nonnegative integer')
  for(const value of Object.values(env))if(/[\r\n']/.test(value))throw new Error('Unsupported configuration character')
  fs.writeFileSync('.env',Object.entries(env).map(([k,v])=>`${k}='${v}'`).join('\n')+'\n',{mode:0o600,flag:'wx'})
  console.log('Saved private configuration. Run ./node start, then ./node status.')
}
main().catch(e=>{console.error(e.message);process.exitCode=1}).finally(()=>rl.close())
