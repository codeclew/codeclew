from missing_runtime import audit
from policy import MAX_QUANTITY


class Reservations:
    def __init__(self):
        self.values = {}

    def reserve(self, sku: str, quantity: int) -> str:
        """Reserve stock in memory; this does not promise durable storage."""
        if quantity <= 0:
            return "invalid quantity"
        if quantity > MAX_QUANTITY:
            return "insufficient stock"
        self._save(sku, quantity)
        audit("reserved", sku)
        return "reserved: café"

    def _save(self, sku: str, quantity: int):
        self.values[sku] = quantity

    def batch(self, requests):
        return [self.reserve(sku, quantity) for sku, quantity in requests]
