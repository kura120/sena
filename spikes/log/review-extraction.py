import asyncio
from cognee.infrastructure.databases.graph import get_graph_engine

async def main():
    engine = await get_graph_engine()
    nodes, edges = await engine.get_graph_data()
    for n in nodes:
        print(n[1].get('type'), '|', n[1].get('name'))

asyncio.run(main())