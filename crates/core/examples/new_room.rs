//! Demo: what the "Chat oluştur" screen does under the hood.
//!
//!     cargo run --release -p ghostline-core --example new_room -- <nickname> [password]

use ghostline_core::{password, KdfParams, Nickname, RoomCredentials};
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let nick = args.next().unwrap_or_else(|| "neo".into());
    let pw = args.next();

    let nick = match Nickname::new(&nick) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("nickname rejected: {e}");
            std::process::exit(1);
        }
    };
    let creds = match RoomCredentials::new_room(pw.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("password rejected: {e}");
            std::process::exit(1);
        }
    };

    println!("> nickname  : {nick}");
    println!("> room id   : {}", creds.id);
    println!("> password  : {}", creds.password.as_str());
    println!(
        "> suggestion: {}   (\"Şifre öner\", ~{:.0} bit)",
        password::generate(password::DEFAULT_LENGTH).as_str(),
        password::generated_entropy_bits(password::DEFAULT_LENGTH)
    );

    let t = Instant::now();
    let secrets = creds.derive(&KdfParams::DESKTOP).expect("kdf");
    println!("> argon2id  : {:?} (m=128MiB t=3 p=4)", t.elapsed());
    println!("> mailbox   : {}", hex(&secrets.mailbox_id));
    println!("> del-hash  : {}", hex(&secrets.delete_token_hash()));
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
