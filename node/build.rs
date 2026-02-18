use sp1_build::build_program_with_args;

fn main() {
    // let args = BuildArgs {
    //     docker: true,
    //     elf_name: Some("energy-tracker-program".to_string()),
    //     ..Default::default()
    // };
    build_program_with_args("../program", Default::default())
}