package paired

import java.util.concurrent.BlockingQueue
import java.util.concurrent.LinkedBlockingQueue

class Body(val type: String, val name: String?, val eligible: Boolean)
class Task(val taskType: String, val type: String, val name: String?, val eligible: Boolean)
class Config(val enabled: Boolean, val prefix: String)
interface Gateway { fun deliver(value: String): Int }

class DispatchEndpoint(private val submitted: BlockingQueue<Task>) {
    fun submit(taskType: String, body: Body): Boolean {
        val task = Task(taskType, body.type, body.name, body.eligible)
        return submitted.offer(task)
    }
}

class ProcessingLoop(
    private val pending: BlockingQueue<Task>,
    private val gateway: Gateway,
    private val config: Config?,
) {
    var lastState = "idle"
    fun runOnce() {
        val task = pending.poll()
        if (task == null) { return }
        val current = config
        if (current == null) { return }
        if (!current.enabled) { return }
        if (!task.eligible) { return }
        val chosen = chooseName(task)
        val transformed = current.prefix + chosen.trim()
        val status = gateway.deliver(transformed)
        if (status != 0) { lastState = "rejected"; return }
        lastState = "sent"
    }
    private fun chooseName(task: Task): String {
        val name = task.name
        if (name == null) { return "anonymous π🙂 @EXT@ .mdx {probe()} <script>" }
        return name
    }
}

class Composition {
    fun assemble(gateway: Gateway, config: Config) {
        val shared = LinkedBlockingQueue<Task>()
        val endpoint = DispatchEndpoint(shared)
        val worker = ProcessingLoop(shared, gateway, config)
    }
}
