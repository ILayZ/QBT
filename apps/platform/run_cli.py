"""Run the CLI in the same process as its Linux parent-death signal."""

import ctypes
import os
import signal
import sys

PR_SET_PDEATHSIG = 1


def main(parent_pid, command):
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(PR_SET_PDEATHSIG, signal.SIGKILL, 0, 0, 0) != 0:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error))
    # Close the race where the worker died before the launcher armed the signal.
    if os.getppid() == parent_pid:
        os.execv(command[0], command)


if __name__ == "__main__":
    main(int(sys.argv[1]), sys.argv[2:])
