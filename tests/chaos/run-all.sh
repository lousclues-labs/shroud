#!/bin/bash
# Chaos Test Runner
# Usage: ./run-all.sh [scenario-number]
# 
# Run all scenarios or a specific one by number

set -e
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "═══════════════════════════════════════════════════════════════════════════"
echo "  SHROUD CHAOS TEST SUITE"
echo "═══════════════════════════════════════════════════════════════════════════"
echo ""
echo "Scenarios available:"
echo ""

# List scenarios
SCENARIOS=($(ls "$SCRIPT_DIR/scenarios/"*.sh 2>/dev/null | sort))

for i in "${!SCENARIOS[@]}"; do
    name=$(basename "${SCENARIOS[$i]}" .sh)
    safety=$(grep -m1 "Safety:" "${SCENARIOS[$i]}" | sed 's/.*Safety: //' || echo "unknown")
    echo "  $((i+1)). $name [$safety]"
done

echo ""

if [[ -n "$1" ]]; then
    # Run specific scenario
    if [[ "$1" =~ ^[0-9]+$ ]] && [[ $1 -ge 1 ]] && [[ $1 -le ${#SCENARIOS[@]} ]]; then
        SCENARIO="${SCENARIOS[$1-1]}"
        echo "Running: $(basename "$SCENARIO")"
        echo ""
        bash "$SCENARIO"
    else
        echo "Invalid scenario number: $1"
        echo "Valid range: 1-${#SCENARIOS[@]}"
        exit 1
    fi
else
    echo "To run a scenario:"
    echo "  $0 <number>     # Run specific scenario"
    echo "  $0 safe         # Run all 🟢 SAFE scenarios"
    echo "  $0 caution      # Run all 🟡 CAUTION scenarios"
    echo "  $0 dangerous    # Run all 🔴 DANGEROUS scenarios (VM only!)"
    echo ""
    
    if [[ "$1" == "safe" ]]; then
        echo "Running all SAFE scenarios..."
        for s in "${SCENARIOS[@]}"; do
            if grep -q "🟢 SAFE" "$s"; then
                echo ""
                echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
                echo "Running: $(basename "$s")"
                echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
                bash "$s"
            fi
        done
    fi
fi
