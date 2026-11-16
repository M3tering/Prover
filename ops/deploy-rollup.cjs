// Authorized Sepolia deployment helper. Never prints keys or RPC URLs.
const fs = require('fs'), path = require('path')
const { JsonRpcProvider, Wallet, ContractFactory, Contract, getCreateAddress } = require('../streamr-client/node_modules/ethers')
const dotenv = require('../streamr-client/node_modules/dotenv')
async function main() {
  const env = dotenv.parse(fs.readFileSync(process.env.DEPLOY_ENV || '.env'))
  const provider = new JsonRpcProvider(env.RPC_URL), signer = new Wallet(env.PRIVATE_KEY, provider)
  if ((await provider.getNetwork()).chainId !== 11155111n) throw new Error('Wrong chain')
  const artifact = JSON.parse(fs.readFileSync(path.resolve(__dirname,'../../rollup/out/Rollup.sol/Rollup.json')))
  const dir = path.resolve(__dirname,'../../rollup/deployments');fs.mkdirSync(dir,{recursive:true})
  const file = path.join(dir,'sepolia.json')
  let record
  if (fs.existsSync(file)) record = JSON.parse(fs.readFileSync(file))
  else {
    const factory = new ContractFactory(artifact.abi, artifact.bytecode.object, signer)
    const tx = await factory.getDeployTransaction()
    const nonce = await signer.getNonce('pending'), gas = await provider.estimateGas({...tx,from:signer.address})
    const fees = await provider.getFeeData()
    const maxFeePerGas=fees.maxFeePerGas, maxPriorityFeePerGas=fees.maxPriorityFeePerGas
    if (!maxFeePerGas || maxFeePerGas > 30000000000n) throw new Error('Fee cap exceeded')
    const gasLimit=gas*120n/100n
    if(await provider.getBalance(signer.address,'pending') < gasLimit*maxFeePerGas+1000000000000000n) throw new Error('Insufficient deployment balance')
    const raw=await signer.signTransaction({...tx,chainId:11155111,nonce,gasLimit,maxFeePerGas,maxPriorityFeePerGas,type:2})
    record={chainId:11155111,address:getCreateAddress({from:signer.address,nonce}),deployer:signer.address,rawDeploymentTransaction:raw,programVKey:'0x00d7ccad99253ce7ee8b85917c04d23c3685645613492931c9ad82efecefe06d',elfSha256:'1cdba7eb2eae2c6839ec69074a9053cbf0cce030cb16d60642e8122145a3fe12'}
    fs.writeFileSync(file,JSON.stringify(record,null,2)+'\n')
  }
  if(await provider.getCode(record.address)==='0x') {
    const sent=await provider.broadcastTransaction(record.rawDeploymentTransaction)
    record.deploymentTransaction=sent.hash;fs.writeFileSync(file,JSON.stringify(record,null,2)+'\n')
    const receipt=await sent.wait(2);if(receipt.status!==1)throw new Error('Deployment reverted')
    record.deploymentBlock=receipt.blockNumber;record.deploymentGas=receipt.gasUsed.toString()
  }
  const rollup=new Contract(record.address,artifact.abi,signer)
  if(await rollup.VKEY_ADMIN()!==signer.address)throw new Error('Unexpected admin')
  if(await rollup.SP1_PROGRAM_VKEY()!==record.programVKey)throw new Error('Unexpected vkey')
  const key='0x2e653ffbcf13dabbf1a41d112171bab4ca1d3bac00e98d80cdce7cce51920608'
  if(await rollup.publicKey(7)!==key) {
    const sent=await rollup.setPublicKey(7,key)
    record.meter7KeyTransaction=sent.hash;fs.writeFileSync(file,JSON.stringify(record,null,2)+'\n')
    const receipt=await sent.wait(2);if(receipt.status!==1)throw new Error('Key registration reverted')
  }
  record.meter7PublicKey=await rollup.publicKey(7)
  record.verifier=await rollup.VERIFIER();record.m3ter=await rollup.M3ter();record.genesisWordCount=String(await rollup.wordCount())
  delete record.rawDeploymentTransaction
  fs.writeFileSync(file,JSON.stringify(record,null,2)+'\n')
  console.log(JSON.stringify(record,null,2))
}
main().catch(e=>{console.error('Deployment stopped:',e.code || e.message);process.exit(1)})
