from solver import median


def test_odd_length():
    # PASS_TO_PASS — the buggy version already handles odd-length lists.
    assert median([3, 1, 2]) == 2


def test_single():
    # PASS_TO_PASS — single element.
    assert median([5]) == 5


def test_even_length():
    # FAIL_TO_PASS — buggy version returns 3; correct median is 2.5.
    assert median([1, 2, 3, 4]) == 2.5
