# Complete timing results

Positive percentages mean less GPU time than the same-frame mean of the two controls.
Paired medians are the primary statistic; means and A/A are retained to show instability.
The disturbed `confirm-main-synthetic` 4K run is excluded from conclusions.

## confirm-main-a

### 1920x1080

A/A difference of mean GPU times: 0.26%.

| Variant | Median ms | Mean ms | Paired median saving | Paired mean saving |
|---|---:|---:|---:|---:|
| base-a | 8.496 | 8.761 | -0.41% | -0.19% |
| base-b | 8.398 | 8.738 | 0.41% | 0.19% |
| direct4 | 6.083 | 6.543 | 25.98% | 24.36% |
| residual4 | 6.097 | 6.709 | 26.05% | 22.47% |
| group5-75 | 5.009 | 5.686 | 39.81% | 34.24% |
| group5-50 | 3.744 | 4.166 | 55.07% | 51.61% |

### 3840x2160

A/A difference of mean GPU times: -0.14%.

| Variant | Median ms | Mean ms | Paired median saving | Paired mean saving |
|---|---:|---:|---:|---:|
| base-a | 21.080 | 21.620 | 0.03% | 0.07% |
| base-b | 21.122 | 21.651 | -0.03% | -0.07% |
| direct4 | 17.007 | 17.399 | 19.59% | 19.55% |
| residual4 | 15.839 | 16.248 | 24.99% | 24.86% |
| group5-75 | 14.642 | 14.955 | 31.00% | 30.86% |
| group5-50 | 9.241 | 9.487 | 56.47% | 56.12% |

## confirm-main-b

### 1920x1080

A/A difference of mean GPU times: -0.05%.

| Variant | Median ms | Mean ms | Paired median saving | Paired mean saving |
|---|---:|---:|---:|---:|
| base-a | 7.516 | 7.587 | 0.04% | 0.03% |
| base-b | 7.529 | 7.591 | -0.04% | -0.03% |
| direct4 | 5.914 | 5.981 | 21.80% | 21.13% |
| residual4 | 5.750 | 5.832 | 23.97% | 23.08% |
| group5-75 | 4.792 | 4.872 | 36.78% | 35.76% |
| group5-50 | 3.538 | 3.611 | 52.96% | 52.36% |

### 3840x2160

A/A difference of mean GPU times: 0.08%.

| Variant | Median ms | Mean ms | Paired median saving | Paired mean saving |
|---|---:|---:|---:|---:|
| base-a | 20.834 | 20.921 | -0.08% | -0.04% |
| base-b | 20.783 | 20.904 | 0.08% | 0.04% |
| direct4 | 16.750 | 16.830 | 19.66% | 19.50% |
| residual4 | 15.613 | 15.681 | 25.21% | 25.00% |
| group5-75 | 14.298 | 14.386 | 31.55% | 31.19% |
| group5-50 | 9.054 | 9.154 | 56.54% | 56.21% |

## confirm-main-synthetic

### 1920x1080

A/A difference of mean GPU times: -0.29%.

| Variant | Median ms | Mean ms | Paired median saving | Paired mean saving |
|---|---:|---:|---:|---:|
| base-a | 9.340 | 9.580 | 0.01% | 0.17% |
| base-b | 9.438 | 9.608 | -0.01% | -0.17% |
| direct4 | 7.184 | 7.491 | 23.11% | 21.45% |
| residual4 | 7.152 | 7.500 | 22.74% | 20.91% |
| group5-75 | 5.923 | 6.267 | 36.75% | 33.79% |
| group5-50 | 4.335 | 4.847 | 53.06% | 48.98% |

### 3840x2160

A/A difference of mean GPU times: -17.59%.

| Variant | Median ms | Mean ms | Paired median saving | Paired mean saving |
|---|---:|---:|---:|---:|
| base-a | 31.216 | 137.141 | 0.02% | -0.63% |
| base-b | 31.595 | 166.410 | -0.02% | 0.63% |
| direct4 | 24.112 | 76.624 | 24.49% | 18.57% |
| residual4 | 23.419 | 158.983 | 25.50% | 10.15% |
| group5-75 | 20.800 | 49.928 | 33.31% | 29.73% |
| group5-50 | 12.394 | 44.507 | 57.58% | 48.20% |

## focused-720

### 926x720

A/A difference of mean GPU times: -1.46%.

| Variant | Median ms | Mean ms | Paired median saving | Paired mean saving |
|---|---:|---:|---:|---:|
| base-a | 3.402 | 3.420 | 0.88% | 0.72% |
| base-b | 3.444 | 3.471 | -0.88% | -0.72% |
| group3-short75 | 2.729 | 2.753 | 20.90% | 19.97% |
| group3-50 | 2.997 | 3.047 | 12.57% | 11.44% |

## focused-repeat

### 1920x1080

A/A difference of mean GPU times: 0.31%.

| Variant | Median ms | Mean ms | Paired median saving | Paired mean saving |
|---|---:|---:|---:|---:|
| base-a | 7.405 | 7.473 | -0.17% | -0.15% |
| base-b | 7.368 | 7.450 | 0.17% | 0.15% |
| group3-short100 | 5.422 | 5.517 | 26.10% | 26.02% |
| group3-short75 | 4.952 | 5.011 | 32.90% | 32.81% |
| group3-short50 | 5.058 | 5.115 | 31.68% | 31.41% |
| group3-50 | 5.553 | 5.629 | 24.66% | 24.52% |

### 3840x2160

A/A difference of mean GPU times: 0.48%.

| Variant | Median ms | Mean ms | Paired median saving | Paired mean saving |
|---|---:|---:|---:|---:|
| base-a | 20.298 | 21.277 | -0.03% | -0.24% |
| base-b | 20.132 | 21.175 | 0.03% | 0.24% |
| group3-short100 | 16.942 | 17.626 | 16.89% | 16.76% |
| group3-short75 | 14.947 | 15.719 | 25.85% | 25.78% |
| group3-short50 | 13.416 | 14.198 | 33.51% | 33.06% |
| group3-50 | 15.460 | 16.345 | 23.44% | 23.03% |

## focused-sparse

### 1920x1080

A/A difference of mean GPU times: -2.05%.

| Variant | Median ms | Mean ms | Paired median saving | Paired mean saving |
|---|---:|---:|---:|---:|
| base-a | 5.915 | 7.351 | 0.21% | 0.83% |
| base-b | 6.038 | 7.505 | -0.21% | -0.83% |
| group3-short75 | 4.431 | 5.745 | 25.67% | 22.28% |
| group3-50 | 4.782 | 6.007 | 23.67% | 18.70% |

### 3840x2160

A/A difference of mean GPU times: -0.08%.

| Variant | Median ms | Mean ms | Paired median saving | Paired mean saving |
|---|---:|---:|---:|---:|
| base-a | 18.971 | 20.148 | 0.01% | 0.05% |
| base-b | 18.966 | 20.164 | -0.01% | -0.05% |
| group3-short75 | 14.102 | 14.861 | 26.20% | 26.23% |
| group3-50 | 14.600 | 15.305 | 24.14% | 24.03% |

## focused-synthetic

### 1920x1080

A/A difference of mean GPU times: 0.52%.

| Variant | Median ms | Mean ms | Paired median saving | Paired mean saving |
|---|---:|---:|---:|---:|
| base-a | 7.623 | 7.621 | -0.40% | -0.26% |
| base-b | 7.499 | 7.582 | 0.40% | 0.26% |
| group3-short75 | 5.156 | 5.196 | 31.67% | 31.62% |
| group3-50 | 5.725 | 5.776 | 24.13% | 24.00% |

### 3840x2160

A/A difference of mean GPU times: -0.35%.

| Variant | Median ms | Mean ms | Paired median saving | Paired mean saving |
|---|---:|---:|---:|---:|
| base-a | 20.433 | 20.558 | -0.06% | 0.13% |
| base-b | 20.392 | 20.630 | 0.06% | -0.13% |
| group3-short75 | 14.984 | 15.053 | 26.56% | 26.84% |
| group3-50 | 15.545 | 15.740 | 23.98% | 23.63% |

## same-look-final

### 1920x1080

A/A difference of mean GPU times: -0.36%.

| Variant | Median ms | Mean ms | Paired median saving | Paired mean saving |
|---|---:|---:|---:|---:|
| base-a | 7.382 | 7.451 | 0.11% | 0.17% |
| base-b | 7.380 | 7.478 | -0.11% | -0.17% |
| micro | 7.234 | 7.321 | 2.06% | 1.84% |
| complete | 7.429 | 7.509 | -0.42% | -0.69% |

### 3840x2160

A/A difference of mean GPU times: 0.99%.

| Variant | Median ms | Mean ms | Paired median saving | Paired mean saving |
|---|---:|---:|---:|---:|
| base-a | 30.541 | 32.930 | -0.61% | -0.56% |
| base-b | 29.839 | 32.608 | 0.61% | 0.56% |
| micro | 29.051 | 31.749 | 2.39% | 2.81% |
| complete | 29.384 | 31.880 | 2.86% | 2.27% |

## screen1-fixed

### 1920x1080

A/A difference of mean GPU times: -1.14%.

| Variant | Median ms | Mean ms | Paired median saving | Paired mean saving |
|---|---:|---:|---:|---:|
| base-a | 8.008 | 8.245 | 0.52% | 0.52% |
| base-b | 8.083 | 8.340 | -0.52% | -0.52% |
| direct4 | 6.226 | 6.363 | 23.82% | 23.04% |
| residual4 | 6.052 | 6.285 | 24.87% | 24.00% |
| s3-a | 7.645 | 7.953 | 5.49% | 3.89% |
| micro | 7.868 | 8.143 | 3.67% | 1.48% |

### 3840x2160

A/A difference of mean GPU times: 0.49%.

| Variant | Median ms | Mean ms | Paired median saving | Paired mean saving |
|---|---:|---:|---:|---:|
| base-a | 24.258 | 23.947 | -0.10% | -0.20% |
| base-b | 23.997 | 23.831 | 0.10% | 0.20% |
| direct4 | 18.672 | 18.886 | 21.25% | 20.87% |
| residual4 | 17.834 | 17.836 | 25.27% | 25.28% |
| s3-a | 23.751 | 23.440 | 1.54% | 1.78% |
| micro | 24.027 | 23.536 | 1.69% | 1.39% |

## screen2-far

### 1920x1080

A/A difference of mean GPU times: -1.43%.

| Variant | Median ms | Mean ms | Paired median saving | Paired mean saving |
|---|---:|---:|---:|---:|
| base-a | 7.477 | 7.577 | 0.69% | 0.72% |
| base-b | 7.607 | 7.687 | -0.69% | -0.72% |
| hybrid-1 | 6.428 | 6.495 | 14.67% | 14.79% |
| hybrid-2 | 6.272 | 6.340 | 17.48% | 16.82% |
| hybrid-3 | 6.697 | 6.755 | 12.51% | 11.37% |
| residual4 | 5.864 | 5.910 | 22.54% | 22.44% |

### 3840x2160

A/A difference of mean GPU times: -1.05%.

| Variant | Median ms | Mean ms | Paired median saving | Paired mean saving |
|---|---:|---:|---:|---:|
| base-a | 21.509 | 24.861 | 0.03% | 0.44% |
| base-b | 21.550 | 25.125 | -0.03% | -0.44% |
| hybrid-1 | 17.643 | 20.544 | 18.12% | 17.63% |
| hybrid-2 | 17.533 | 20.231 | 18.48% | 18.82% |
| hybrid-3 | 17.994 | 20.482 | 17.56% | 17.62% |
| residual4 | 16.277 | 19.242 | 24.48% | 23.17% |

## screen2-hybrids

### 1920x1080

A/A difference of mean GPU times: -0.25%.

| Variant | Median ms | Mean ms | Paired median saving | Paired mean saving |
|---|---:|---:|---:|---:|
| base-a | 8.986 | 9.262 | 0.12% | 0.18% |
| base-b | 8.925 | 9.285 | -0.12% | -0.18% |
| hybrid-4 | 7.686 | 7.988 | 14.26% | 13.31% |
| hybrid-8 | 7.660 | 7.932 | 15.14% | 14.08% |
| hybrid-12 | 8.417 | 8.652 | 6.39% | 6.06% |
| hybrid-16 | 7.039 | 7.147 | 22.12% | 22.26% |
| hybrid-24 | 7.648 | 8.124 | 13.58% | 12.24% |
| hybrid-28 | 8.319 | 8.642 | 6.40% | 6.23% |

### 3840x2160

A/A difference of mean GPU times: -0.75%.

| Variant | Median ms | Mean ms | Paired median saving | Paired mean saving |
|---|---:|---:|---:|---:|
| base-a | 25.707 | 26.013 | 0.48% | 0.45% |
| base-b | 25.674 | 26.209 | -0.48% | -0.45% |
| hybrid-4 | 23.526 | 23.735 | 8.03% | 8.34% |
| hybrid-8 | 23.710 | 23.492 | 8.53% | 9.33% |
| hybrid-12 | 26.339 | 26.423 | -2.33% | -2.04% |
| hybrid-16 | 20.921 | 21.293 | 19.04% | 17.78% |
| hybrid-24 | 23.131 | 23.663 | 9.55% | 8.61% |
| hybrid-28 | 25.977 | 26.033 | -0.67% | -0.52% |

## screen3-groups

### 1920x1080

A/A difference of mean GPU times: -1.03%.

| Variant | Median ms | Mean ms | Paired median saving | Paired mean saving |
|---|---:|---:|---:|---:|
| base-a | 7.484 | 7.542 | 0.40% | 0.51% |
| base-b | 7.533 | 7.620 | -0.40% | -0.51% |
| group3-75 | 6.161 | 6.198 | 18.33% | 18.15% |
| group3-50 | 5.740 | 5.783 | 23.97% | 23.66% |
| group5-75 | 4.747 | 4.856 | 36.77% | 35.89% |
| group5-50 | 3.542 | 3.598 | 53.22% | 52.49% |

### 3840x2160

A/A difference of mean GPU times: 0.04%.

| Variant | Median ms | Mean ms | Paired median saving | Paired mean saving |
|---|---:|---:|---:|---:|
| base-a | 20.713 | 20.775 | -0.01% | -0.02% |
| base-b | 20.655 | 20.768 | 0.01% | 0.02% |
| group3-75 | 19.191 | 19.187 | 7.62% | 7.62% |
| group3-50 | 15.897 | 15.948 | 23.38% | 23.21% |
| group5-75 | 14.147 | 14.236 | 31.68% | 31.46% |
| group5-50 | 9.028 | 9.097 | 56.43% | 56.20% |

## screen4-combinations

### 1920x1080

A/A difference of mean GPU times: -0.24%.

| Variant | Median ms | Mean ms | Paired median saving | Paired mean saving |
|---|---:|---:|---:|---:|
| base-a | 7.851 | 7.824 | -0.10% | 0.12% |
| base-b | 7.861 | 7.843 | 0.10% | -0.12% |
| group3-50 | 5.877 | 5.987 | 24.32% | 23.44% |
| group3-short75 | 5.169 | 5.296 | 33.86% | 32.22% |
| group3-short50 | 5.286 | 5.475 | 31.75% | 29.99% |
| hybrid-24 | 6.623 | 6.756 | 14.61% | 13.59% |

### 3840x2160

A/A difference of mean GPU times: -1.02%.

| Variant | Median ms | Mean ms | Paired median saving | Paired mean saving |
|---|---:|---:|---:|---:|
| base-a | 21.601 | 21.623 | 0.14% | 0.40% |
| base-b | 21.701 | 21.845 | -0.14% | -0.40% |
| group3-50 | 16.770 | 16.708 | 23.17% | 22.97% |
| group3-short75 | 15.944 | 15.952 | 26.33% | 26.45% |
| group3-short50 | 14.446 | 14.502 | 33.08% | 33.18% |
| hybrid-24 | 19.619 | 19.682 | 9.50% | 9.37% |
