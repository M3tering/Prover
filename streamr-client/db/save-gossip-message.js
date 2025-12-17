
const pgp = require('pg-promise')()
const decodePayload = require('../util/decode').decodePayload

async function saveGossipMessage(db, messages) {
   console.log("Received messages to save:", messages);
   const cs = new pgp.helpers.ColumnSet([
      'm3ter_id', 'nonce', 'energy', 'message', 'signature', 'is_verified'], 
      { table: 'm3ter_payloads' }
   );

   const mapped = await Promise.all(messages
      .map(async msg => {
         const { nonce, energy, message, signature } = decodePayload(msg.message);
         const exist = await db.oneOrNone(
            'SELECT id FROM m3ter_payloads WHERE m3ter_id = $1 AND nonce = $2',
            [msg.m3ter_id, nonce]
         );
         if (exist) return null     

         return {
            m3ter_id: msg.m3ter_id,
            nonce,
            energy,
            message,
            signature,
            is_verified: false
         };
      }))
   const values = mapped.filter(Boolean)
      
   if (values.length === 0) {
      console.log("No new messages to save");
      return;
   }

   const query = pgp.helpers.insert(values, cs);
   await db.none(query);
   console.log("💾 Saving batch messages:", values.length);
}

module.exports = {
   saveGossipMessage
}