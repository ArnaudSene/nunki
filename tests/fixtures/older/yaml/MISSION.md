---
branch: feat/1_000
base: dev
lots:
- id: L1
  title: 2026-10-13
- id: L2
  title: 'no: not yes, # really'
integration:
  kind: services
  services:
  - name: db
    reach:
    - db
    - 10.4.0.7
  wiring: []
security: agent
rigor: critical
mutation_threshold: 90
arbiter: 'null'
account: null
model: null
run: null
bounds:
  max_volets: 3
  attempts_per_lot: 3
  checkpoint_minutes: 45
  check_minutes: 15
  stall_checks: 3
  long_lot_hours: 8
  mutation_minutes: 45
  harness_wait_hours: 6
  five_hour_stop_percent: 90
  weekly_stop_percent: 80
---

# Mission

Do the thing.
