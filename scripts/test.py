#!/usr/bin/env python3

import subprocess
import os
import sys
import difflib

root_dir = os.path.abspath(os.path.join(__file__, os.pardir, os.pardir))
test_dir = os.path.join(root_dir, "tests")

green = "\x1b[32m"
red = "\x1b[31m"
yellow = "\x1b[33m"
reset = "\x1b[0m"

passed = 0
failed = 0
skipped = 0


def get_lines(bytes, file_path):
    expected_lines = []
    with open(file_path, "r") as contents:
        expected_lines = contents.read().splitlines()
        actual_lines = bytes.decode().splitlines()
    return expected_lines, actual_lines


def print_pretty_diff(a, b):
    sys.stdout.write("│ │\n")

    def write(lines, color=None):
        for line in lines:
            if color == green:
                sys.stdout.write(f"│ │ {green}+{reset} {line}\n")
            elif color == red:
                sys.stdout.write(f"│ │ {red}-{reset} {line}\n")
            else:
                sys.stdout.write(f"│ │   {line}\n")

    matcher = difflib.SequenceMatcher(None, a, b)
    for opcode, a0, a1, b0, b1 in matcher.get_opcodes():
        if opcode == "equal":
            write(a[a0:a1])
        elif opcode == "insert":
            write(b[b0:b1], green)
        elif opcode == "delete":
            write(a[a0:a1], red)
        elif opcode == "replace":
            write(b[b0:b1], green)
            write(a[a0:a1], red)


filter_arg = None
if len(sys.argv) > 1:
    filter_arg = sys.argv[1]
    sys.stdout.write(f"Running tests matching '{filter_arg}'...\n")
else:
    sys.stdout.write("Running all tests...\n")

for entry in os.scandir(test_dir):
    if entry.is_dir():
        test_module = os.path.join(test_dir, entry.name, "main.pa")

        test_cases = [
            # ("run.out", "run", lambda c: c.stdout),
            # ("run.err", "run", lambda c: c.stderr),
            ("analyze.out", "analyze", lambda c: c.stdout),
            ("analyze.err", "analyze", lambda c: c.stderr),
        ]

        for output_file, command, handler in test_cases:
            output_file_path = os.path.join(test_dir, entry.name, output_file)
            test_name = f"{entry.name}/{output_file}"

            if filter_arg is not None and filter_arg not in test_name:
                skipped += 1
                continue

            sys.stdout.write(f"\n{test_name}: ")

            if not os.path.exists(output_file_path):
                sys.stdout.write(f"{yellow}missing{reset}\n")
                skipped += 1
                continue

            command = subprocess.run(
                ["cargo", "run", "--bin", "cli", "--quiet", "--", command, test_module],
                capture_output=True,
            )

            expected_lines, actual_lines = get_lines(handler(command), output_file_path)

            if expected_lines == actual_lines:
                sys.stdout.write(f"{green}passed{reset}\n")
                passed += 1
            else:
                failed += 1
                sys.stdout.write(f"{red}failed{reset}\n")
                sys.stdout.write("│\n│ Incorrect or unexpected output:\n")
                print_pretty_diff(expected_lines, actual_lines)

sys.stdout.write(f"\n{green}{passed} passed{reset}\n")
sys.stdout.write(f"{red}{failed} failed{reset}\n")
sys.stdout.write(f"{yellow}{skipped} skipped{reset}\n")

if failed > 0:
    exit(1)

if skipped > 0:
    exit(2)
