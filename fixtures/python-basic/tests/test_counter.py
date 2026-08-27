import unittest

from counter import Counter


class CounterTest(unittest.TestCase):
    def test_increment_uses_one_unit_step(self) -> None:
        self.assertEqual(Counter(3).increment(), 4)
