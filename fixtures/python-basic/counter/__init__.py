class Counter:
    def __init__(self, value: int) -> None:
        self.value = value

    def increment(self) -> int:
        self.value += 1
        return self.value
