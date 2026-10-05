#!/usr/bin/env python3
"""Create the public Pricing saved-edit case in a new caller-owned directory.

Requires a Clew source checkout and Git. Does not run Clew, Gradle or a model.
"""
import argparse
from pathlib import Path
import shutil
import subprocess


BASE = """package fixture
object Pricing {
    fun price(): Int = 1
    fun label(x: Int): String = x.toString()
}
fun consumer(): Int = Pricing.price() + 1
fun labelConsumer(): String = Pricing.label(1)
fun stopCalling(): Int = Pricing.price()
fun main() { Pricing.price() }
fun stable(): Int = 9
fun documented(): Int { /* old comment */ return 7 }
"""


def prepare(checkout, output):
    fixture = checkout.resolve() / 'fixtures/kotlin-basic'
    if not fixture.is_dir() or not (fixture / 'gradlew').is_file():
        raise ValueError('Select a Clew source checkout containing fixtures/kotlin-basic/gradlew.')
    if output.exists() or output.is_symlink():
        raise ValueError('Destination already exists. Choose a new directory; nothing was replaced.')
    shutil.copytree(fixture, output, ignore=shutil.ignore_patterns(
        '.git', '.gradle', 'build', '.semantic-thread'))
    shutil.rmtree(output / 'src')
    source = output / 'src/main/kotlin/Price.kt'
    source.parent.mkdir(parents=True)
    source.write_text(BASE)
    test = output / 'src/test/kotlin/PriceTest.kt'
    test.parent.mkdir(parents=True)
    test.write_text('package fixture\nimport kotlin.test.Test\nimport kotlin.test.assertEquals\n'
                    'class PriceTest { @Test fun priceCheck() { assertEquals(1, Pricing.price()) } }\n')
    (source.parent / 'Deleted.kt').write_text('package fixture\nfun removed(): Int = 4\n')
    original = source.parent / 'Original.kt'
    original.write_text('package fixture\nfun moved(): Int = 6\n')

    def git(*args):
        subprocess.run(['git', *args], cwd=output, check=True, capture_output=True)

    git('init', '-q', '-b', 'main')
    git('add', '.')
    git('-c', 'user.name=Public fixture', '-c', 'user.email=test@codeclew.invalid',
        '-c', 'commit.gpgsign=false', 'commit', '-qm', 'Pricing base')
    source.write_text(BASE.replace('= 1\n', '= 2\n'))
    git('add', '.')
    source.write_text(BASE.replace('= 1\n', '= 3\n').replace('x: Int', 'x: Long')
                      .replace('label(1)', 'label(1L)').replace('old comment', 'new comment')
                      .replace('stopCalling(): Int = Pricing.price()', 'stopCalling(): Int = 0'))
    (source.parent / 'Deleted.kt').unlink()
    original.rename(source.parent / 'Renamed.kt')
    (source.parent / 'Added.kt').write_text('package fixture\nfun added(): Int = 5\n')
    return output.resolve()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--checkout', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    try:
        output = prepare(args.checkout, args.output)
    except (ValueError, subprocess.CalledProcessError) as error:
        parser.exit(1, f'{error}\n')
    print(f'Prepared {output}\nHEAD price: 1; staged price: 2; saved price: 3. No analysis or tests executed.')


if __name__ == '__main__':
    main()
