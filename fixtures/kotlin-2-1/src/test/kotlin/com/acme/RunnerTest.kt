package com.acme

class RunnerTest {
    fun `applies configured limit`() {
        check(applyOptions("record", Options(3)) == "rec")
    }
}
