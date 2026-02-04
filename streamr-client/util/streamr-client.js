
const { StreamrClient }  = require('@streamr/sdk');

const { STREAM_ID } = process.env;

if (!STREAM_ID) {
   throw new Error("Missing STREAM_ID in env");
}

// Initialize the client with an Ethereum account
const streamr = new StreamrClient({
   // environment: process.env.STREAMR_ENV == "live" ? "polygon" : "polygonAmoy",
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