"""Regenerate float16_fma.tsv using exact Fraction arithmetic (stdlib only)."""
from fractions import Fraction
from pathlib import Path
import random


def pow2(exponent):
    return Fraction(2**exponent) if exponent >= 0 else Fraction(1, 2**-exponent)


def decode(bits, fraction, bias):
    magnitude = bits & 0x7fff
    exponent = magnitude >> fraction
    mantissa = magnitude & ((1 << fraction) - 1)
    if exponent:
        mantissa |= 1 << fraction
    value = mantissa * pow2((exponent or 1) - bias - fraction)
    return -value if bits & 0x8000 else value


def nearest(value):
    quotient, remainder = divmod(value.numerator, value.denominator)
    return quotient + (2 * remainder > value.denominator or
                       (2 * remainder == value.denominator and quotient & 1))


def quantize(value, fraction, bias, infinity):
    sign = 0x8000 if value < 0 else 0
    value = abs(value)
    if value < pow2(1 - bias):
        return sign | nearest(value / pow2(1 - bias - fraction))
    exponent = value.numerator.bit_length() - value.denominator.bit_length()
    if value < pow2(exponent):
        exponent -= 1
    mantissa = nearest(value / pow2(exponent - fraction))
    if mantissa == 1 << (fraction + 1):
        exponent += 1
        mantissa >>= 1
    if exponent > bias:
        return sign | infinity
    return sign | ((exponent + bias) << fraction) | (mantissa - (1 << fraction))


def fma(a, b, c, fraction, bias, infinity):
    x, y, z = (bits & 0x7fff for bits in (a, b, c))
    nan = infinity | (1 << (fraction - 1))
    if max(x, y, z) > infinity or (x == infinity and y == 0) or (y == infinity and x == 0):
        return nan
    sign = (a ^ b) & 0x8000
    if x == infinity or y == infinity:
        return nan if z == infinity and sign != c & 0x8000 else sign | infinity
    if z == infinity:
        return c
    result = decode(a, fraction, bias) * decode(b, fraction, bias) + decode(c, fraction, bias)
    if result == 0:
        return sign & c & 0x8000
    return quantize(result, fraction, bias, infinity)


randomizer = random.Random(20261029)
rows = ["# format a b c result (hex); exact Fraction oracle; regenerate with this directory's generate_float16_fma.py"]
for name, fraction, bias, infinity in [("f16", 10, 15, 0x7c00), ("bf16", 7, 127, 0x7f80)]:
    one = bias << fraction
    one_half = one | (1 << (fraction - 1))
    edge = [0, 0x8000, 1, 0x8001, one, one | 0x8000, infinity - 1,
            infinity, infinity | 0x8000, infinity | 1, 0xffff]
    triples = [(a, b, c) for a in edge for b in edge for c in edge]
    triples += [tuple(randomizer.randrange(65536) for _ in range(3)) for _ in range(2048)]
    # Midpoint products perturbed in both directions, including exponent gaps
    # too large for either f32 or f64 fused arithmetic to keep the perturbation.
    for offset in [1, 3, 5, 7]:
        for tiny in [1, 0x8001, (bias - 40) << fraction if bias > 40 else 1]:
            triples.append((one + offset, one_half, tiny))
            triples.append(((one + offset) | 0x8000, one_half, tiny))
    for a, b, c in triples:
        expected = fma(a, b, c, fraction, bias, infinity)
        rows.append(f"{name} {a:04x} {b:04x} {c:04x} {expected:04x}")
Path(__file__).with_name("float16_fma.tsv").write_text("\n".join(rows) + "\n")
