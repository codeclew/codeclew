import { consumer } from "./client.js";

interface ParcelMessage {
    topic: string;
    partition: number;
    message: { value: string | null, offset: string };
}

declare function reserve(value: string): Promise<boolean>;

let processed = 0;

export async function processParcel({ topic, partition, message }: ParcelMessage) {
    if (!message.value) return;
    if (await reserve(message.value)) processed += 1;
    await consumer.commitOffsets([{ topic, partition, offset: message.offset }]);
}
