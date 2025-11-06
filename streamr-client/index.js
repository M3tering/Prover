
const express = require('express')
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

initializeDatabase().then(db => {
    const app = express()
    const port = process.env.PORT || 3000

    app.get('/', res => {
        res.send('Hello World!')
    })

    streamr.subscribe(STREAM_ID, data => {
       console.log("📥 Received message:", data)

       // Type guard to ensure data is a StreamrMessage
       if (
          data &&
          typeof data === "array"
       ) {
          saveGossipMessage(db, data)
       } else {
          console.warn("⚠️ Received invalid message format:", data)
       }
    })

    app.listen(port, () => {
        console.log(`Server is running on port ${port}`)
    })
})