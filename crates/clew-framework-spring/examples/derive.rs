//! Test bridge: compiler fixtures exercise the same pure Rust interpreter.
use std::io::{Read, Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = Vec::new();
    std::io::stdin()
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut input)?;
    if input.len() > 16 * 1024 * 1024 {
        return Err("fixture input exceeds its bound".into());
    }
    let inputs: Vec<clew_facts::JvmAnnotationFacts> = serde_json::from_slice(&input)?;
    let outputs = inputs
        .iter()
        .map(clew_framework_spring::analyze)
        .collect::<Result<Vec<_>, _>>()?;
    std::io::stdout().write_all(&serde_json::to_vec(&outputs)?)?;
    Ok(())
}
