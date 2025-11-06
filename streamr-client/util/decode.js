
function decodePayload(buf) {
   if (buf.length < 72) {
      throw new Error("Payload too short. Must be at least 72 bytes");
   }
   const nonce = buf.readUInt32BE(0);

   const rawEnergy = buf.readUInt32BE(4);
   const energyKWh = rawEnergy / 1e6;

   const signature = buf.subarray(8, 72).toString("hex");

   return {
      nonce,
      energy: energyKWh,
      signature,
   };
}

module.exports = {
   decodePayload,
};