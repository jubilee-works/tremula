"""Locate the tremula binary bundled with this wheel."""

import os
import sysconfig

__all__ = ["find_tremula_bin"]


def find_tremula_bin() -> str:
    """Return the absolute path to the tremula binary installed with this package.

    Raises:
        FileNotFoundError: If the binary is not present in the scripts directory.
    """
    name = "tremula" + (sysconfig.get_config_var("EXE") or "")
    path = os.path.join(sysconfig.get_path("scripts"), name)
    if not os.path.isfile(path):
        raise FileNotFoundError(path)
    return path
