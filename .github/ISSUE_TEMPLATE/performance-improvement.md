---
name: Performance improvement
about: Report a performance bottleneck or propose an optimization with benchmarks
  and profiling data.
title: ''
labels: R-NeedsTriage, T-Performance
assignees: ''

---

**Summary**

Briefly describe the performance issue or the proposed optimization.

**Affected Component / Area**

Which module, function, or workflow is affected?

**Current Behavior & Metrics**

Describe the current performance characteristics. Include concrete numbers where possible (e.g., latency, throughput, FPS, memory usage, CPU utilization, build/startup time).

- Metric (e.g., latency / memory / throughput):
- Observed value:
- Expected / target value:

**Reproduction / Benchmark Steps**

Provide minimal steps, a benchmark script, or a workload to reproduce and measure the bottleneck.

1.
2.
3.

**Profiling & Root Cause Analysis (Optional)**

Attach or describe any profiling results (e.g., flamegraphs, trace logs, memory snapshots, query plans) and what you believe is causing the bottleneck.

**Environment**

- Platform:
- Version / Commit SHA: (If this is a performance regression, please provide two versions)
- Hardware specs (CPU / RAM / GPU):

**Proposed Solution (Optional)**

If you have an idea for how to optimize this, describe the approach and any potential trade-offs (e.g., memory vs. speed, code complexity).
