package client

import example.JavaNormalizer

class KotlinClient {
    fun normalize(input: String): String = JavaNormalizer().normalize(input)

    fun selectedOverload(value: Int): Int = JavaNormalizer().overloaded(value)
}
