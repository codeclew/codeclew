import { consumer } from "./client.js";
import { processParcel } from "./parcel.js";

export async function start() {
    await consumer.subscribe({ topic: "parcels.ready" });
    await consumer.run({ autoCommit: false, eachMessage: processParcel });
}

setInterval(async () => {
    const samples = await loadHealth();
    for (const sample of samples) {
        const key = sample.id;
        await updateHealth(key);
        {
            const key = "independent";
            await updateHealth(key);
        }
    }
}, 5000);

declare function loadHealth(): Promise<{ id: string }[]>;
declare function recordHealth(key: string): Promise<void>;

async function updateHealth(key: string) {
    await recordHealth(key);
}

function unrelated(setInterval: (callback: () => void) => void) {
    setInterval(() => { throw new Error("shadowed callback"); });
}
