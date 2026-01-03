
const express = require('express')
const events = require('events')
const dotenv = require('dotenv')

dotenv.config()

const { streamr, STREAM_ID } = require('./util/streamr-client')
const { saveGossipMessage } = require('./db/save-gossip-message')

const pgp = require('pg-promise')()
const MigrationManager = require('./db/migration-manager')

async function initializeDatabase() {
    const db = pgp(process.env.DATABASE_URL)
    const migrationManager = new MigrationManager(db)

    try {
        await migrationManager.runMigrations()
        console.log('Migrations completed successfully')
    } catch (error) {
        console.error('Migration failed:', error)
        process.exit(1)
    }

    return db
}

initializeDatabase().then(async db => {

    events.setMaxListeners(20)
    const app = express()
    const port = process.env.STREAMR_CLIENT_PORT || 3000

    app.get('/health', res => {
        res.status(200).json({ status: 'ok' })
    })

    let sub = await streamr.subscribe(STREAM_ID, data => {
       if (
          data &&
          Array.isArray(data)
       ) {
          saveGossipMessage(db, data)
       } else {
          console.warn("⚠️ Received invalid message format:", data)
       }
    })

    console.log(`Subscribed to stream: ${sub.streamPartId}`)

    sub.on('error', (err) => {
        console.error('Subscription error:', err)
    })

    // Clean up subscription on process exit
    process.on('SIGINT', async () => {
        try {
            await sub.unsubscribe()
            process.exit(0)
        } catch (err) {
            console.error('Error during cleanup:', err)
            process.exit(1)
        }
    })

    app.listen(port, () => {
        console.log(`Server is running on port ${port}`)
    })
})