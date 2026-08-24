mod helper;

pub fn format_message(name: &str) -> String {
    format!("hello, {name}")
}

pub use helper::Formatter;
