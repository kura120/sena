import asyncio
import cognee

async def main():
    await cognee.prune.prune_data()
    await cognee.prune.prune_system(metadata=True)
    await cognee.add("The Sena project uses LadybugDB as its graph database.")
    await cognee.cognify()

asyncio.run(main())