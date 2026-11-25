import { consumer } from "./client.js";
import { processParcel } from "./parcel.js";

export async function start() {
    await consumer.subscribe({ topic: "parcels.ready" });
    await consumer.run({ autoCommit: false, eachMessage: processParcel });
}

setInterval(async () => { await updateHealth(); }, 5000);
declare function updateHealth(): Promise<void>;

function unrelated(setInterval: (callback: () => void) => void) {
    setInterval(() => { throw new Error("shadowed callback"); });
}
