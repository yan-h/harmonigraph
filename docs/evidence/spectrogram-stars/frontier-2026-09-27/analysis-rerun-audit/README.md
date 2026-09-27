# Timing analysis rerun audit

Prior per-size analyses were preserved under `prior/` before rerunning the current analyzer.

Compared 55 files; 3 changed.

Changed paired savings: 6; bootstrap intervals: 6; A/A values: 3; order/count fields: 0.

No changed metrics were discarded; see `diff-summary.json` for old and new values. Interval and savings units are percent as stored. The current analyzer computes the paired ratio of aggregate means and expresses block-bootstrap interval endpoints in percent.

A/A screening (absolute mean A/B difference greater than 3 percentage points) flags `separate-control/3840x2160` at -9.09% and `compute-half/1920x1080` at -3.19%. This is a descriptive screening threshold, not a significance test. The missing calibration 4K triplet is an expected acquisition gap, not an analyzer failure.
