
const { StreamPermission, StreamrClient }  = require('@streamr/sdk');

const { STREAM_ID, PRIVATE_KEY } = process.env;

if (!STREAM_ID || !PRIVATE_KEY) {
   throw new Error("Missing STREAMR_PRIVATE_KEY or STREAM_ID in env");
}

// Initialize the client with an Ethereum account
const streamr = new StreamrClient({
   auth: {
      privateKey: PRIVATE_KEY,
   },
   environment: process.env.STREAMR_ENV == "live" ? "polygon" : "polygonAmoy",
});

async function ensureStream() {
   const stream = await streamr.getOrCreateStream({ id: STREAM_ID });
   return stream;
}

module.exports = {
   streamr,
   ensureStream,
   STREAM_ID
};