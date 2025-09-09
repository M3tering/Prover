use sp1_sdk::{include_elf, HashableKey, Prover, ProverClient};

/// The ELF (executable and linkable format) file for the Succinct RISC-V zkVM.
pub const ENERGY_TRACKER_ELF: &[u8] = include_elf!("energy-tracker-program");

fn main() {
    let prover = ProverClient::builder().cpu().build();
    let (_, vk) = prover.setup(ENERGY_TRACKER_ELF);
    println!("{}", vk.bytes32());
}