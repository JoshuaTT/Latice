import gateway


def test_package_imports() -> None:
    """The skeleton package must import and document itself."""
    assert gateway.__doc__
