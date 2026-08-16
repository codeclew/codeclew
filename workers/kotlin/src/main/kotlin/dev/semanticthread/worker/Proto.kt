package dev.semanticthread.worker

import java.io.EOFException
import java.io.InputStream
import java.io.OutputStream

internal object Proto {
    data class Field(val number: Int, val wire: Int, val bytes: ByteArray = byteArrayOf(), val value: Long = 0)

    fun fields(bytes: ByteArray): List<Field> {
        val out = mutableListOf<Field>()
        var i = 0
        while (i < bytes.size) {
            val (tag, next) = readVarint(bytes, i); i = next
            val number = (tag ushr 3).toInt(); val wire = (tag and 7).toInt()
            when (wire) {
                0 -> { val (v, n) = readVarint(bytes, i); i = n; out += Field(number, wire, value = v) }
                2 -> {
                    val (len, n) = readVarint(bytes, i); i = n
                    require(len >= 0 && i + len.toInt() <= bytes.size) { "invalid protobuf length" }
                    out += Field(number, wire, bytes.copyOfRange(i, i + len.toInt())); i += len.toInt()
                }
                else -> error("unsupported protobuf wire type $wire")
            }
        }
        return out
    }

    fun message(vararg fields: ByteArray): ByteArray = fields.fold(byteArrayOf()) { a, b -> a + b }
    fun uint(number: Int, value: Long) = varint((number shl 3).toLong()) + varint(value)
    fun bytes(number: Int, value: ByteArray) = varint(((number shl 3) or 2).toLong()) + varint(value.size.toLong()) + value
    fun string(number: Int, value: String) = bytes(number, value.toByteArray(Charsets.UTF_8))

    private fun varint(input: Long): ByteArray {
        var value = input; val out = ArrayList<Byte>()
        do { var b = (value and 0x7f).toInt(); value = value ushr 7; if (value != 0L) b = b or 0x80; out += b.toByte() } while (value != 0L)
        return out.toByteArray()
    }

    private fun readVarint(bytes: ByteArray, start: Int): Pair<Long, Int> {
        var result = 0L; var shift = 0; var i = start
        while (i < bytes.size && shift < 64) {
            val b = bytes[i++].toInt() and 0xff; result = result or ((b and 0x7f).toLong() shl shift)
            if (b and 0x80 == 0) return result to i
            shift += 7
        }
        error("invalid protobuf varint")
    }

    fun readFrame(input: InputStream): ByteArray? {
        val header = ByteArray(4); var read = 0
        while (read < 4) { val n = input.read(header, read, 4 - read); if (n < 0) { if (read == 0) return null else throw EOFException() }; read += n }
        val length = ((header[0].toInt() and 0xff) shl 24) or ((header[1].toInt() and 0xff) shl 16) or ((header[2].toInt() and 0xff) shl 8) or (header[3].toInt() and 0xff)
        require(length in 0..(64 * 1024 * 1024)) { "invalid frame length $length" }
        return input.readNBytes(length).also { if (it.size != length) throw EOFException() }
    }

    fun writeFrame(output: OutputStream, bytes: ByteArray, maximumFrameBytes: Int = 64 * 1024 * 1024) {
        val n = bytes.size
        require(maximumFrameBytes >= 0 && n <= maximumFrameBytes) { "protobuf frame exceeds the bounded transport limit" }
        output.write(byteArrayOf((n ushr 24).toByte(), (n ushr 16).toByte(), (n ushr 8).toByte(), n.toByte()))
        output.write(bytes); output.flush()
    }
}
