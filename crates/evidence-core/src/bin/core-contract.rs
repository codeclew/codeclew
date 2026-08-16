use std::path::PathBuf;

use evidence_core::FrozenCoreContract;

fn main() {
    if let Err(error) = run() {
        eprintln!("core-contract: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let command = arguments.next().unwrap_or_else(|| "verify".to_owned());
    let mut write = false;
    let mut root = None;
    for argument in arguments {
        if argument == "--write" {
            write = true;
        } else if root.replace(PathBuf::from(argument)).is_some() {
            return Err("only one repository root may be provided".into());
        }
    }
    let root = root.unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    match command.as_str() {
        "freeze" => {
            let contract = FrozenCoreContract::compute(&root)?;
            if write {
                contract.write_lock(&root)?;
            }
            println!("{}", String::from_utf8(contract.canonical_bytes()?)?);
        }
        "verify" if !write => {
            let contract = FrozenCoreContract::verify(&root)?;
            println!("{}", String::from_utf8(contract.canonical_bytes()?)?);
        }
        "verify" => return Err("--write is valid only with freeze".into()),
        _ => return Err("usage: core-contract [verify|freeze] [--write] [repository-root]".into()),
    }
    Ok(())
}
