import pilot


def test_package_imports() -> None:
    """The skeleton package must import and document itself."""
    assert pilot.__doc__
