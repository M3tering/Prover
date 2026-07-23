use std::env;

use diesel::{
    PgConnection, r2d2::{self, ConnectionManager, PooledConnection}, table, update,
};
use energy_tracker_lib::decode_slice;

type DbPool = r2d2::Pool<ConnectionManager<PgConnection>>;


table! {
    m3ter_payloads (id) {
        id -> Int4,
        m3ter_id -> Int8,
        message -> VarChar,
        signature -> VarChar,
        nonce -> Int8,
        energy -> Int8,
        is_verified -> Bool,
    }
}

pub fn establish_db_connection() -> DbPool {
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL not set in .env");
    let manager = ConnectionManager::<PgConnection>::new(&database_url);
    r2d2::Pool::builder()
        .build(manager)
        .expect("Failed to create pool.")
}

pub async fn update_payload(
    connection: &mut PooledConnection<ConnectionManager<PgConnection>>,
    nonces: Vec<u8>,
) {
    use self::m3ter_payloads::dsl::*;
    use diesel::prelude::*;

    let nonces_in_db = m3ter_payloads
        .select(m3ter_id)
        .distinct()
        .load::<i64>(connection)
        .unwrap();
    for m3ter in nonces_in_db {
        let n = m3ter as usize;
        let start = n * 6;
        let end = n * 6 + 6;
        let nonce_value = decode_slice(&nonces[start..end].try_into().unwrap()) as i64;

        let rows_deleted = update(m3ter_payloads)
            .filter(m3ter_id.eq(m3ter))
            .filter(nonce.le(3362i64))
            .set(is_verified.eq(true))
            .execute(connection)
            .unwrap();

        println!("rows deleted {}", rows_deleted);
        assert!(nonce_value > 100)
    }
    // println!("nonces in db {:?}", nonces_in_db)
}
// // update payloads
//             let _ = diesel::update(
//                 m3ter_payloads.filter(m3ter_id.eq(i as i64).and(nonce.le(nonce_filter))),
//             )
//             .set(is_verified.eq(true))
//             .execute(connection)
//             .expect("Failed to update payloads");

//             // delete payloads
//             let _ = diesel::delete(
//                 m3ter_payloads.filter(m3ter_id.eq(i as i64).and(nonce.eq(nonce_filter))),
//             )
//             .execute(connection)
//             .expect("Failed to delete payloads");
