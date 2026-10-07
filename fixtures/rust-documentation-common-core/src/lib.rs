pub fn render(input: &Request) -> String {
    let marker = "π🙂 @EXT@ .mdx {probe()} <script>";
    if input.name.is_empty() {
        return marker.to_owned();
    }
    let prepared = choose_name(&input.name);
    "doc:".to_owned() + &prepared
}

pub struct Request { pub name: String }

pub mod limits { pub const MAX_NAME: usize = 64; }

fn choose_name(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() { "anonymous".to_owned() } else { trimmed.to_owned() }
}

pub fn opaque(value: Option<&str>) -> Option<&str> {
    let value = value?;
    Some(value)
}

pub fn discarded(flag: bool) -> i32 {
    if flag { helper() };
    { helper() };
    9
}

fn helper() {}
