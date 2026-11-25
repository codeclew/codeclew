import { Kafka } from "kafkajs";

const kafka = new Kafka({ brokers: ["broker:9092"] });
export const consumer = kafka.consumer({ groupId: "parcel-processing" });
