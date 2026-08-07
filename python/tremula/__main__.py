"""Allow `python -m tremula` to behave like the tremula binary."""

import os
import sys

from tremula import find_tremula_bin

if __name__ == "__main__":
    bin_path = find_tremula_bin()
    os.execv(bin_path, [bin_path, *sys.argv[1:]])
