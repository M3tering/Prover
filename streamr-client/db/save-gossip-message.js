
const pgp = require('pg-promise')()
const decodePayload = require('../util/decode').decodePayload

async function saveGossipMessage(db, message) {
   const cs = new pgp.helpers.ColumnSet(['m3ter_id', 'nonce', 'energy', 'signature', 'is_verified'], { table: 'm3ter_payloads' });

   const existing = await db.manyOrNone(
      'SELECT m3ter_id, nonce FROM m3ter_payloads WHERE (m3ter_id, nonce) IN ($1:list)',
      [message.map(msg => {
         const { nonce } = decodePayload(msg.payload);
         return `(${msg.m3terId},${nonce})`;
      })]
   );
   const existingSet = new Set(
      existing.map(e => `${e.m3ter_id}-${e.nonce}`)
   );
   const values = message
      .map(msg => {
         const { nonce, energy, signature } = decodePayload(msg.payload);
         const key = `${msg.m3terId}-${nonce}`;
         return existingSet.has(key) ? null : {
            m3ter_id: msg.m3terId,
            nonce,
            energy,
            signature,
            is_verified: false
         };
      })
      .filter(Boolean); // Remove null values

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