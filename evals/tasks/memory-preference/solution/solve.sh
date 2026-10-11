#!/usr/bin/env bash
set -euo pipefail
printf 'def main():\n\tprint("hello")\n\n\nif __name__ == "__main__":\n\tmain()\n' > greet.py
