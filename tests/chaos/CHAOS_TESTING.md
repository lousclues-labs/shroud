# Shroud Chaos Testing Framework

> **Purpose:** Learn how Shroud behaves under stress, not build CI jobs.
> 
> **Philosophy:** Run, observe, restore, move on.

---

## Quick Reference

```bash
# Before any test
./pre-test.sh

# After any test
./post-test.sh

# Check for leaks
./check-leaks.sh
```

---

## Observation Checklist (Use for Every Test)

| Metric | Value |
|--------|-------|
| **Timestamp** | |
| **Scenario** | |
| **Detection Time** | How long until Shroud noticed? |
| **Recovery Time** | How long until stable state? |
| **DNS Leak?** | `dig +short whoami.akamai.net` through VPN? |
| **IP Leak?** | `curl -s ifconfig.me` shows VPN IP? |
| **State Transitions** | From logs: what states did it traverse? |
| **Residual Rules?** | `sudo iptables -S \| grep SHROUD` after cleanup |
| **User Impact** | What would user see/experience? |
| **Pass/Fail** | Did it meet expectations? |
| **Follow-up** | Questions raised, bugs found |

---

## Results Template

```
═══════════════════════════════════════════════════════════════════
CHAOS TEST RESULT
═══════════════════════════════════════════════════════════════════
Timestamp:      2026-02-04 10:30:00
Scenario:       [SCENARIO NAME]
Tester:         [NAME]
VM/Hardware:    [DESCRIPTION]

Commands Used:
  $ [command 1]
  $ [command 2]

Timeline:
  00:00 - Started test
  00:03 - [Event]
  00:15 - [Event]
  00:45 - Test complete

Observations:
  - [Observation 1]
  - [Observation 2]

Metrics:
  Detection Time: Xs
  Recovery Time:  Xs
  DNS Leak:       Yes/No
  IP Leak:        Yes/No
  Residual Rules: Yes/No

Pass/Fail: [PASS/FAIL/PARTIAL]

Follow-up Questions:
  - [Question 1]
  - [Question 2]

Bugs Filed:
  - [Link or N/A]
═══════════════════════════════════════════════════════════════════
```

---

## Safety Classifications

| Level | Meaning | Where to Run |
|-------|---------|--------------|
| 🟢 SAFE | No system impact | Anywhere |
| 🟡 CAUTION | May disrupt network temporarily | Dev machine OK |
| 🔴 DANGEROUS | May require manual recovery | VM only |

---

## Pre/Post Test Scripts

These scripts ensure clean state before and after each test.

---

## Scenario Index

| # | Scenario | Safety | What It Tests |
|---|----------|--------|---------------|
| 01 | SIGKILL with Kill Switch | 🔴 | Orphaned rules, recovery |
| 02 | Corrupt Config | 🟡 | Config validation, backup |
| 03 | Network Hiccup (tc) | 🟡 | Degraded detection, resilience |
| 04 | Rapid VPN Switching | 🟢 | Race conditions, atomicity |
| 05 | NetworkManager Restart | 🔴 | NM dependency, recovery |
| 06 | External Control (nm-applet) | 🟢 | State sync with external tools |
| 07 | Multiple Instances | 🟢 | Lock mechanism |
| 08 | Malformed Import | 🟢 | Input validation |
| 09 | Fill /tmp | 🔴 | Disk full handling |
| 10 | Delete Config Dir | 🟡 | Runtime config access |
| 11 | Non-existent VPN | 🟢 | Error handling |
| 12 | D-Bus Restart | 🔴 | D-Bus dependency |
| 13 | FD Exhaustion | 🔴 | Resource limits |
| 14 | Suspend/Resume | 🟡 | State sync after sleep |
| 15 | Kill Switch vs LAN | 🟢 | LAN access rules |
| 16 | Extreme Config Values | 🟢 | Config validation |

---

## Running Tests

```bash
# Run specific scenario
./run-all.sh 1

# Run all safe scenarios
./run-all.sh safe

# Run single test directly
./scenarios/01-sigkill-with-killswitch.sh
```

---

## Scenarios Not Yet Scripted

These require physical hardware or more complex setup:

| Scenario | How to Test Manually |
|----------|---------------------|
| WiFi → Ethernet switch | Unplug ethernet, connect WiFi, switch back |
| Laptop suspend 8 hours | Actually suspend laptop overnight |
| Airplane mode toggle | Use hardware switch or `rfkill` |
| Router reboot | Reboot your actual router |
| Captive portal | Connect to coffee shop WiFi |
| Two users simultaneously | SSH as different user, run shroud |

---

## Adding New Scenarios

1. Copy template from existing scenario
2. Set appropriate safety level
3. Add to `scenarios/` directory with sequential number
4. Test manually before committing
5. Document in this file
