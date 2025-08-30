#!/bin/bash
#
# Benchmark Runner Script for GSH Performance Testing
# 
# This script runs all benchmarks and generates HTML reports for analysis
#

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "🚀 Starting GSH Performance Benchmark Suite..."
echo "============================================="

# Check if criterion HTML feature is available
if ! cargo --list | grep -q "bench"; then
    echo "❌ Cargo bench command not found. Please install Rust with benchmark support."
    exit 1
fi

# Function to run benchmarks with proper error handling
run_benchmark() {
    local dir="$1"
    local name="$2"
    
    echo "📊 Running $name benchmarks..."
    cd "$dir" || exit 1
    
    if cargo bench --quiet 2>/dev/null; then
        echo "✅ $name benchmarks completed successfully"
    else
        echo "⚠️  $name benchmarks encountered issues but continuing..."
    fi
    
    cd - >/dev/null || exit 1
}

# Run libgsh internal benchmarks
if [ -d "libgsh" ]; then
    run_benchmark "libgsh" "LibGSH Internal"
fi

# Run comprehensive benchmark suite
if [ -d "benches" ]; then
    echo ""
    echo "📊 Running comprehensive benchmark suite..."
    cd benches || exit 1
    
    # Individual benchmark categories
    echo "  🖥️  Server Performance..."
    cargo bench --bench server_performance --quiet || echo "    ⚠️  Server performance benchmarks had issues"
    
    echo "  🎞️  Frame Processing..."
    cargo bench --bench frame_processing --quiet || echo "    ⚠️  Frame processing benchmarks had issues"
    
    echo "  🔒 Authentication..."
    cargo bench --bench authentication --quiet || echo "    ⚠️  Authentication benchmarks had issues"
    
    # Placeholder benchmarks (will be expanded later)
    echo "  🌐 Client-Server Communication..."
    cargo bench --bench client_server_communication --quiet || echo "    ⚠️  Communication benchmarks had issues"
    
    echo "  📈 Concurrent Load Testing..."
    cargo bench --bench concurrent_load --quiet || echo "    ⚠️  Load testing benchmarks had issues"
    
    cd - >/dev/null || exit 1
fi

echo ""
echo "📈 Benchmark Results Summary"
echo "============================"

# Find and display benchmark result locations
if find . -name "criterion" -type d 2>/dev/null | head -1 >/dev/null; then
    echo "📁 Benchmark results saved to:"
    find . -name "criterion" -type d | while read -r dir; do
        echo "   📊 $(realpath "$dir")"
        # Look for HTML reports
        if find "$dir" -name "index.html" 2>/dev/null | head -1 >/dev/null; then
            echo "      🌐 HTML report: $(find "$dir" -name "index.html" | head -1)"
        fi
    done
else
    echo "⚠️  No benchmark results found. This may be normal for first run."
fi

echo ""
echo "✅ Benchmark suite completed!"
echo ""
echo "💡 Tips:"
echo "   • Open HTML reports in your browser to view detailed performance analysis"
echo "   • Run 'cargo bench' in individual directories for focused testing"
echo "   • Use 'cargo bench -- --quick' for faster runs during development"
echo "   • Compare results over time to track performance regressions"