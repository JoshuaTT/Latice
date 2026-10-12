import orchestrator


def test_package_imports() -> None:
    """The skeleton package must import and document itself."""
    assert orchestrator.__doc__
