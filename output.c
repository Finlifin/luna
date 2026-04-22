#include <stdint.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>

typedef int64_t Int;
typedef double Float;
typedef bool Bool;
typedef char Char;
typedef const char* String;

typedef struct {
    Int x;
    Int y;
} Adt_4;

Int add(Int a, Int b);
Int factorial(Int n);
Int fib(Int n);
Int point_sum(Int a, Int b);
Int dist_sq(Int x1, Int y1, Int x2, Int y2);

Int add(Int a, Int b) {
    Int _0;
    Int _3;

bb0:
    _3 = (a + b);
    _0 = _3;
    return _0;

}

Int factorial(Int n) {
    Int _0;
    Bool _2;
    Int _3;
    Int _4;
    Int _5;
    Int _6;

bb0:
    _2 = (n == 0);
    if (_2) goto bb1; else goto bb2;

bb1:
    _3 = 1;
    goto bb3;

bb2:
    _4 = (n - 1);
    _5 = factorial(_4);
    goto bb4;

bb3:
    _0 = _3;
    return _0;

bb4:
    _6 = (n * _5);
    _3 = _6;
    goto bb3;

}

Int fib(Int n) {
    Int _0;
    Bool _2;
    Int _3;
    Int _4;
    Int _5;
    Int _6;
    Int _7;
    Int _8;

bb0:
    _2 = (n <= 1);
    if (_2) goto bb1; else goto bb2;

bb1:
    _3 = n;
    goto bb3;

bb2:
    _4 = (n - 1);
    _5 = fib(_4);
    goto bb4;

bb3:
    _0 = _3;
    return _0;

bb4:
    _6 = (n - 2);
    _7 = fib(_6);
    goto bb5;

bb5:
    _8 = add(_5, _7);
    goto bb6;

bb6:
    _3 = _8;
    goto bb3;

}

Int point_sum(Int a, Int b) {
    Int _0;
    Adt_4 _3;
    Adt_4 _4;
    Int _5;
    Int _6;
    Int _7;

bb0:
    _4 = (Adt_4){.x = a, .y = b};
    _3 = _4;
    _5 = _3.x;
    _6 = _3.y;
    _7 = (_5 + _6);
    _0 = _7;
    return _0;

}

Int dist_sq(Int x1, Int y1, Int x2, Int y2) {
    Int _0;
    Adt_4 _5;
    Adt_4 _6;
    Adt_4 _7;
    Adt_4 _8;
    Int _9;
    Int _10;
    Int _11;
    Int _12;
    Int _13;
    Int _14;
    Int _15;
    Int _16;
    Int _17;
    Int _18;
    Int _19;

bb0:
    _6 = (Adt_4){.x = x1, .y = y1};
    _5 = _6;
    _8 = (Adt_4){.x = x2, .y = y2};
    _7 = _8;
    _10 = _5.x;
    _11 = _7.x;
    _12 = (_10 - _11);
    _9 = _12;
    _14 = _5.y;
    _15 = _7.y;
    _16 = (_14 - _15);
    _13 = _16;
    _17 = (_9 * _9);
    _18 = (_13 * _13);
    _19 = (_17 + _18);
    _0 = _19;
    return _0;

}

int main(void) {
    printf("add(3, 4) = %lld\n", (long long)add(3, 4));
    printf("factorial(10) = %lld\n", (long long)factorial(10));
    printf("fib(10) = %lld\n", (long long)fib(10));
    printf("point_sum(3, 4) = %lld\n", (long long)point_sum(3, 4));
    printf("dist_sq(1, 5, 4, 2) = %lld\n", (long long)dist_sq(1, 5, 4, 2));
    return 0;
}
