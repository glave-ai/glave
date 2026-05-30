from intervals import merge


def test_simple_overlap():
    # PASS_TO_PASS — plain overlapping intervals.
    assert merge([[1, 3], [2, 4]]) == [[1, 4]]


def test_no_overlap():
    # PASS_TO_PASS — disjoint intervals are left alone.
    assert merge([[1, 2], [5, 6]]) == [[1, 2], [5, 6]]


def test_nested_interval():
    # FAIL_TO_PASS — a nested interval must not shrink the merged range.
    assert merge([[1, 10], [2, 3]]) == [[1, 10]]
