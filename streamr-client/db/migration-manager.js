const pgp = require('pg-promise')();
const path = require('path');
const fs = require('fs').promises;

class MigrationManager {
   constructor(db) {
      this.db = db;
   }

   async initialize() {
      await this.db.none(`
            CREATE TABLE IF NOT EXISTS migrations (
                id SERIAL PRIMARY KEY,
                name VARCHAR(255) NOT NULL,
                applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )
        `);
   }

   async getMigrationsToRun() {
      const applied = await this.db.map(
         'SELECT name FROM migrations',
         [],
         row => row.name
      );

      const files = await fs.readdir(path.join(__dirname, './migrations'));
      return files
         .filter(f => f.endsWith('.sql'))
         .filter(f => !applied.includes(f));
   }

   async runMigrations() {
      await this.initialize();
      const migrations = await this.getMigrationsToRun();

      for (const migration of migrations) {
         try {
            const sql = await fs.readFile(
               path.join(__dirname, './migrations', migration),
               'utf8'
            );

            await this.db.tx(async t => {
               await t.none(sql);
               await t.none(
                  'INSERT INTO migrations(name) VALUES($1)',
                  migration
               );
            });

            console.log(`Applied migration: ${migration}`);
         } catch (error) {
            console.error(`Failed to apply migration ${migration}:`, error);
            throw error;
         }
      }
   }
}

module.exports = MigrationManager;