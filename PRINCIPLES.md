# Shroud — Core Principles

These principles define how Shroud is built and must be reflected in all documentation, code comments, and design decisions.

---

## I. Wrap, Don't Replace

Shroud is not a VPN. It is the armor around one.

We do not reinvent NetworkManager. We do not rewrite OpenVPN or WireGuard. We do not spawn daemons where none are needed. The Linux ecosystem already solved these problems — our job is to protect and enhance, not to compete.

When you wrap something, you respect its shape. Shroud follows the contours of the tools it surrounds.

---

## II. Fail Loud, Recover Quiet

When something breaks, the user must know. No silent failures. No "connected" lies while packets fall into the void.

But recovery should be graceful. Reconnect without fanfare. Restore state without drama. The user should feel the safety net catch them — not hear it creak.

Silence is for success. Alarms are for failure.

---

## III. Leave No Trace

When Shroud stops, it stops completely.

No orphaned firewall rules. No ghost routes. No zombie sockets. No "run this command to fix your networking after uninstall."

A tool that doesn't clean up after itself is a tool that doesn't respect its host. We are guests in this system.

---

## IV. The User Is Not the Enemy

No telemetry. No phoning home. No analytics. No "anonymous usage data."

If the user wants to run Shroud in a bunker with no internet except the VPN tunnel, that is their right. We exist to protect privacy, not to erode it from the inside.

Trust is not a feature. It is the foundation.

---

## V. Complexity Is Debt

Every dependency is a liability. Every abstraction is a potential failure point. Every line of code is a maintenance burden.

If it can be done with nmcli, do not write a D-Bus binding. If it can be done with iptables, do not shell out to legacy tooling. If it can be done in one crate, do not import three.

Simplicity is not laziness. It is discipline.

---

## VI. Speak the System's Language

We use NetworkManager because it is there. We use D-Bus because it is the lingua franca. We use XDG paths because that is where config belongs. We use iptables because it is the current kill switch engine.

Shroud should feel native on any Linux system — not like a foreign ambassador demanding special accommodations.

When in Rome, use systemd.

---

## VII. State Is Sacred

The state machine is not an implementation detail. It is the source of truth.

If the state says Disconnected, we are disconnected. If the state says Degraded, we are degraded. We do not guess. We do not assume. We do not check "just to be sure" and then ignore what we find.

Every transition has a reason. Every reason is logged. Ambiguity is a bug.

---

## VIII. One Binary, One Purpose

Shroud is a single Rust binary. Not a daemon. Not a client-server pair. Not a collection of microservices pretending to be a desktop app.

It starts. It runs. It stops. That's it.

If you can't explain what it does in one sentence, it's doing too much.

---

## IX. Respect the Disconnect

Sometimes the user wants to be offline. Sometimes the VPN should stay down. Sometimes "do nothing" is the correct action.

Shroud does not nag. It does not auto-connect without permission. It does not treat disconnection as a problem to be solved.

The user's intent is sovereign.

---

## X. Built for the Quiet Majority

Most Linux users don't file bug reports. They just stop using software that breaks.

Shroud is for the user who doesn't want to debug. Who doesn't want to read the wiki. Who wants to click "connect" and have it work — today, tomorrow, after a kernel update, after suspend, after three weeks of uptime.

We are not building for the enthusiast who enjoys the fight. We are building for the professional who needs it to disappear into the background.

Boring is the goal.

---

## XI. Security Through Clarity

A kill switch that the user doesn't understand is a kill switch that the user will disable.

Every rule Shroud applies should be auditable. Every decision should be explainable. If the user asks "why is my LAN blocked?" there should be an answer that doesn't require reading source code.

Obscurity is not security. Clarity is trust.

---

## XII. We Ship, Then Improve

Perfection is the enemy of protection.

A working kill switch today is better than an elegant one next month. A basic tray icon that shows status is better than a beautiful one that's still in Figma.

Shroud exists to solve a real problem for real users. Every day without a release is a day someone's traffic leaks.

Ship the shield. Polish it later.

---

*These principles are not rules. They are promises — to the user, to the system, and to ourselves.*

*When in doubt, ask: "Does this make Shroud more like a shroud, or more like NordVPN?"*

*The answer should be obvious.*
