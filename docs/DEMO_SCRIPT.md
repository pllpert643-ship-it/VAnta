# VAnta Demo Video Script

Target duration: **2–3 minutes**

## 1. The problem — 15 seconds

Show two GPU tasks being launched without a controller or use a simple title
card illustrating both tasks competing for limited VRAM.

Voiceover:

> On a workstation, GPU jobs are often started independently. When limited
> VRAM is shared, users get contention, unpredictable ordering, and little
> explanation of what happened.

## 2. Introduce VAnta — 20 seconds

Open `VAnta.exe`. Point to the live GPU, VRAM, CPU, and RAM cards.

> VAnta is a user-level GPU workload controller for Windows. It does not modify
> the NVIDIA driver or Windows scheduler. It controls only jobs submitted
> through the application.

Highlight the controller loop:

> Every decision follows Observe, Rule, Queue or Action, and Verify.

## 3. Show priority scheduling — 50 seconds

1. Click **Run demo scenario**. It submits a Balanced workload first, then a
   Critical workload, followed by a VRAM guard check.
2. Show that the Critical job waits while the Normal job continues.
3. Keep the queue visible while the guard check is held behind the higher
   priority work.

> VAnta uses a predictable non-preemptive policy. The current job is safe from
> forced suspension, while the critical job moves ahead of other waiting jobs.
> The decision column explains that the concurrency limit is active.

## 4. Show verification — 30 seconds

Point to the chart, VRAM card, and Decision feed while the first job runs.

> After launch, VAnta measures the machine again. The decision feed reports the
> observed VRAM change, giving us evidence that the action had its intended
> effect.

Wait for the first job to finish and show the Critical job start next.

## 5. Show the safety rule — 25 seconds

After the first two demo jobs finish, the **VRAM guard check** is rejected
because its requested VRAM is larger than the safe capacity.

If GPU telemetry is unavailable, the rule engine explains that it is using its
concurrency-only fallback; the guard check then serves as a normal process
lifecycle example instead of a VRAM rejection.

> The rule engine rejects a request that cannot fit while preserving the 512 MB
> safety reserve. The user sees the exact reason instead of a mysterious crash.

## 6. Close — 15 seconds

Show the complete dashboard and repository title card.

> VAnta turns local GPU execution into a transparent, measurable workflow. It
> provides a practical control layer today and a foundation for multi-GPU and
> distributed scheduling tomorrow.

End card:

**VAnta — See the workload. Explain the decision. Control the queue.**

## Optional Policy Lab shot — 30 seconds

Open **Policy Lab** from the launch card. Show the same three-job scenario under
the two policies:

> Strict priority keeps the critical job ahead of the small preview, even while
> VRAM is temporarily unavailable. Fit what fits starts the preview sooner and
> reports the lower wait time. VAnta makes that scheduling trade-off visible
> before changing the real queue policy.
