
function decodePayload(msg) {
   const buf = Buffer.from(msg, 'hex');
   if (buf.length < 72) {
      throw new Error("Payload too short. Must be at least 72 bytes");
   }
   const nonce = buf.readUInt32BE(0);
   const energy = buf.readUInt32BE(4);

   const message = buf.subarray(0, 8).toString("hex");
   const signature = buf.subarray(8, 72).toString("hex");

   return {
      nonce,
      energy,
      message,
      signature,
   };
}

module.exports = {
   decodePayload,
};



