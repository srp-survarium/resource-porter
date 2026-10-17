#!/usr/bin/env bash
# Exit 0 when a Survarium game client runs (its Wine process, not a shell that names it).
ps -eo args= | grep -qE '^([A-Z]:\\|start\.exe /exec ).*(survarium-capture|survarium_skhid|survarium_srp)[^ ]*\.exe'
