#![no_main]
sp1_zkvm::entrypoint!(main);
use m3tering_protocol::transition::{execute, Input};
pub fn main() {
    let input = sp1_zkvm::io::read::<Input>();
    let transition = execute(&input).expect("invalid state transition witness");
    sp1_zkvm::io::commit_slice(transition.public_values.as_slice());
}
