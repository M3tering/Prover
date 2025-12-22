const path = require('path')
const fs = require('fs').promises

const ADVISORY_LOCK_ID = 987654321 // arbitrary but constant

class MigrationManager {
   constructor(db, migrationsDir = path.resolve(process.cwd(), 'db/migrations')) {
      this.db = db
      this.migrationsDir = migrationsDir
   }

   async initialize() {
      await this.db.none(`
      CREATE TABLE IF NOT EXISTS migrations (
        id SERIAL PRIMARY KEY,
        name VARCHAR(255) NOT NULL UNIQUE,
        applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
      )
    `)
   }

   async getAppliedMigrations() {
      return this.db.map(
         'SELECT name FROM migrations',
         [],
         row => row.name
      )
   }

   async getMigrationsToRun() {
      const applied = new Set(await this.getAppliedMigrations())

      const files = (await fs.readdir(this.migrationsDir))
         .filter(f => f.endsWith('.sql'))
         .sort()

      return files.filter(file => !applied.has(file))
   }

   isNonTransactional(sql) {
      return /CREATE\s+DATABASE|CREATE\s+EXTENSION/i.test(sql)
   }

   async applyMigration(file) {
      const sql = await fs.readFile(
         path.join(this.migrationsDir, file),
         'utf8'
      )

      const insert = `
      INSERT INTO migrations(name)
      VALUES($1)
    `

      if (this.isNonTransactional(sql)) {
         await this.db.none(sql)
         await this.db.none(insert, file)
      } else {
         await this.db.tx(async t => {
            await t.none(sql)
            await t.none(insert, file)
         })
      }

      console.log(`✔ Applied migration: ${file}`)
   }

   async runMigrations() {
      await this.initialize()

      // prevent concurrent migration runners
      await this.db.none(
         'SELECT pg_advisory_lock($1)',
         ADVISORY_LOCK_ID
      )

      try {
         const migrations = await this.getMigrationsToRun()

         if (migrations.length === 0) {
            console.log('✓ No pending migrations')
            return
         }

         for (const file of migrations) {
            await this.applyMigration(file)
         }
      } finally {
         await this.db.none(
            'SELECT pg_advisory_unlock($1)',
            ADVISORY_LOCK_ID
         )
      }
   }
}

module.exports = MigrationManager
