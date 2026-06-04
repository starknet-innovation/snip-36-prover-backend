// Tiny fixture task for the packaged-deps smoke test (scripts/smoke-test-deps.sh).
// Proven through the bundled bootloader by stwo-run-and-prove --verify in CI.
// Keep it small: a few hundred VM steps, output + range_check builtins.
%builtins output range_check

from starkware.cairo.common.math import assert_nn
from starkware.cairo.common.serialize import serialize_word

func fib(first: felt, second: felt, n: felt) -> felt {
    if (n == 0) {
        return second;
    }
    return fib(second, first + second, n - 1);
}

func main{output_ptr: felt*, range_check_ptr}() {
    let f = fib(1, 1, 20);
    assert_nn(f);
    serialize_word(f);
    return ();
}
