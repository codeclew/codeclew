from example.service import Service


async def test_service_normalizes_input() -> None:
    service = Service()
    assert await service.execute(" VALUE ") == "value!"
