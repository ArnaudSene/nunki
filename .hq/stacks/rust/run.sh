#!/bin/sh
# How an application of this stack is started (SPEC 4.2, "les services et le
# lancement de l'application").
#
# `hq` runs this, detached, from the root of the tree, inside the container
# of the role that will test or attack the application — before that role is
# launched. No agent starts the application; this script is how the project
# says what starting it means.
#
# It is called as `run.sh <id>`. Three things it must do, and the first two
# are how `hq` knows the application is up at all:
#
#  - keep the id on its command line. `hq` recognises this process by it, and
#    that is why the command below is **not** `exec`ed: replacing the process
#    replaces its command line, and the application would be reported as
#    stopped one second after it started (measured, 2026-09-10).
#  - stay in the foreground. A script that forks and exits reports an
#    application that is not running.
#  - listen on the address the mission's services can reach, not only on
#    127.0.0.1, when something outside the container has to reach it.
#
# Replace the command below with whatever starting this project means. A
# project with nothing to start says `run: none` in hq.yaml instead.
set -eu

cargo run --release
