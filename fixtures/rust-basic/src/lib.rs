pub struct Counter {
    value: i32,
}

impl Counter {
    pub fn new(value: i32) -> Self {
        Self { value }
    }

    pub fn increment(&mut self) -> i32 {
        self.value += 1;
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::Counter;

    #[test]
    fn increment_uses_one_unit_step() {
        assert_eq!(Counter::new(3).increment(), 4);
    }
}
