# VAnta Microsoft Store readiness

Research date: **2026-11-17**. Decision: **not ready for submission** on the available evidence. This is a readiness assessment, not a Microsoft certification decision. Native Rust/Win32 utilities have supported submission routes; utility functionality alone does not establish rejection. [1][2][3]

## Evidence and scope

Parent inspection confirmed the local release executable reports Authenticode `NotSigned`; repository searches found no established Store package manifest, installer, or privacy documentation. The supplied release is a portable EXE/ZIP. Reported validation consists of 67 native tests, 136 Python tests, and local GUI smoke testing, with no clean-machine tests or actual administrator integration testing. These are parent/user-supplied findings, not checks rerun by this researcher. Native administrator operations, system-cache operations, and undocumented APIs require review. `StartupApproved` writes and NVIDIA integration are specific review targets supplied by the parent; their exact behavior still needs a source audit.

Only official Microsoft documentation was used for requirements. Browser-tool calls returned empty results; successful direct HTTPS retrieval of the official pages below supplied the evidence. The retrieved policy identifies **version 7.20**, published September 15, 2026, effective October 22, 2026. This version/date was independently checked after the parent's earlier unverified observation. [1]

## Submission routes and signing

| Topic | Packaged MSIX | Unpackaged EXE/MSI |
|---|---|---|
| Submission artifact | A supported MSIX/AppX package, bundle, or upload format, with a valid package manifest and Store identity. Manual packaging is supported; Rust does not require adopting Visual Studio as the compiler. [3] | A direct HTTPS URL to an actual `.exe` or `.msi` installer. A portable executable is not automatically an installer. [2] |
| Signing | Microsoft re-signs MSIX/AppX packages after certification. A purchased CA-trusted certificate, CA PFX/CER, or hardware token is **not required for this Store submission route**. Outside-Store deployment has separate signing requirements. [3] | The Store does not re-sign the installer. The installer and all its PE files must carry code-signing trust chaining to a CA in the Microsoft Trusted Root Program. The currently unsigned release does not meet this requirement. [2][3] |
| Installation and hosting | Store package deployment; Microsoft documents free signing/CDN hosting and integrated updates as benefits. Validate packaged behavior and manifest identity. [2][3] | Standalone offline installer, silent installation, with installer UAC permitted. Host it yourself at a versioned HTTPS URL; never replace the binary at a submitted URL. Supply a new URL for updated binaries. [2] |
| Main VAnta decision | Prove compatibility and resolve elevation/capability review before investing in packaging. [4][5] | Build and test the installer, signing, hosting, upgrade, and removal process; this route still undergoes Store review. [2][6] |

A **portable ZIP is not a submission artifact** for either route: it is neither a supported Store package nor the EXE/MSI installer accepted through the URL route. Keep portable distribution separate from the Store candidate. [2][3]

## System operations, elevation, and dependencies

Store policy section 10.2.8 requires supported methods and user consent for changes to Windows settings or the Windows experience; it specifically addresses unsupported use of undocumented APIs. **VAnta assessment:** audit startup control, including `StartupApproved`, and cache/system operations individually. Record their supported API contract, permissions, effects, consent, and recovery. Replace or remove unsupported Windows-setting changes from the candidate unless Microsoft clarifies their acceptability. Consent alone does not establish that an implementation is supported. This is a concrete compliance risk, not a conclusion that every system utility is prohibited. [1, section 10.2.8]

For MSIX, the shared capability reference distinguishes medium-integrity `runFullTrust` from `allowElevation`. Full trust does not itself grant administrator elevation. `allowElevation` supports certain existing elevated desktop functionality for Microsoft partners or enterprise organizations, and Store approval is subject to strict criteria. Restricted-capability use must be explained in Partner Center and can fail certification if approval is denied. Do not assume declaring a capability guarantees acceptance. [4]

There is a documentation conflict: the desktop-to-MSIX preparation page broadly says Store apps requiring elevation will not be accepted, while the shared capability reference documents a strictly reviewed `allowElevation` path. Treat MSIX elevation as unresolved for VAnta and obtain Microsoft clarification for the proposed design. EXE/MSI installer UAC permission establishes installer behavior; it does not independently approve every privileged runtime operation. [2][4][5]

Policy section 10.2.4 addresses external dependencies, generally disallowing non-Microsoft drivers or NT services with possible case-by-case consideration for WHCP-certified drivers. It requires driver/service dependencies in certification notes, and primary-function dependencies at the beginning of the listing description. **VAnta assessment:** determine whether NVIDIA functionality is optional or essential, disclose its driver/hardware requirement accurately, and show graceful behavior without it. Do not equate optional use of an already installed vendor API with a proven requirement to distribute a driver. [1, section 10.2.4]

## Privacy, certification, and testing

Policy section 10.5.1 generally conditions privacy-policy requirements on personal-information access/collection/transmission or applicable law, then explicitly says Desktop Bridge and Win32 products inherently accessing personal information must always have privacy policies. **For VAnta's Win32 submission, provide a truthful public privacy-policy URL.** This is not a claim that every app category universally requires one. The EXE/MSI submission checklist's conditional privacy field does not remove the policy's explicit Win32 provision. Document actual local inspection, logs, network activity, retention, sharing, and controls; do not claim "no data collection" without auditing behavior. [1, section 10.5.1][6]

Certification includes policy compliance and testability, not just compilation or unit-test totals. Provide reproducible reviewer instructions for privileged actions, unavailable hardware, and supported limitations. The EXE/MSI workflow requires submission for certification after the required sections are complete. [1, section 10.3][6]

Microsoft explicitly tells MSIX submitters to run the Windows App Certification Kit (WACK). The kit supports Desktop Bridge testing and reports problems locally; passing it is evidence, not certification approval. Run the checks applicable to the chosen artifact and retain their reports. Do not represent WACK as a universally mandatory EXE/MSI upload field: that route's documented checklist asks for installer URL and metadata. [3][6][7]

**Recommended VAnta release evidence:** test the exact distributable on clean Windows machines/VMs without developer tools, including install, first launch, upgrade, uninstall, reboot where applicable, standard-user behavior, elevation approval/denial, partial failure, missing NVIDIA support, and actual administrator operations. Use disposable machines for system changes, verify intended effects and recovery, and cover every OS/architecture claimed in the listing. The existing tests and smoke run do not establish these results. This test matrix is an engineering recommendation based on the current gaps, not a verbatim Microsoft checklist. [3][7]

## Prioritized checklist

1. **P0 - resolve product eligibility risks.** Audit the supplied privileged/undocumented operations and optional NVIDIA paths. Produce a per-feature supported-method and consent record; remove or replace risky behavior and obtain Microsoft clarification where needed. Resolve the MSIX elevation question before committing to that route. [1][4][5]
2. **P0 - choose and build one submission artifact.** For EXE/MSI, establish a silent offline installer, CA-trusted signatures for installer/all PEs, immutable versioned HTTPS hosting, and tested upgrades/removal. For MSIX, establish package identity, manifest, assets, supported architecture/OS declarations, and capability justification; use Store signing requirements rather than purchasing a certificate unnecessarily. [2][3][4]
3. **P0 - close validation gaps.** Complete the clean-machine and administrator matrix above; for MSIX run WACK against the final package. Retain build/artifact identifiers and reports. [3][7]
4. **P1 - prepare privacy and reviewer disclosures.** Publish the audited privacy policy; explain elevation, system modifications, and driver/service dependencies. Provide concise certification steps and expected results. [1][4][6]
5. **P1 - prepare Partner Center and listing.** Establish an active developer account, reserve the product name, set markets/pricing/discoverability, category and age ratings, and fill the route's required package fields. For EXE/MSI provide URL, architecture, language, installer type and silent parameters; complete at least one listing with description, required screenshot/logo and license terms. Describe hardware dependencies and limitations accurately. Use route-specific requirements rather than copying the EXE/MSI form wholesale into MSIX. [3][6]
6. **P2 - submit the tested candidate.** Supply reviewer notes and any restricted-capability explanations, submit for certification, inspect the report, and address findings before public release. Listing completion and local passing tests do not promise approval. [4][6]

**Conditional recommendation:** EXE/MSI appears the more direct candidate if VAnta retains conventional desktop installation and administrator workflows, provided its operations comply with policy and the installer/signing/testing gaps are closed. Prefer MSIX if a packaging proof demonstrates supported behavior and any elevation needs are removed or explicitly approved. Neither route is ready now; changing the container alone will not resolve unsupported operations or missing integration evidence. [1][2][3][4][5]

## Official sources

All URLs accessed successfully by direct HTTPS on 2026-11-17. Clause references above refer to the retrieved policy version, not a certification outcome.

1. Microsoft Store Policies - especially sections 10.2.4, 10.2.8, 10.2.9, 10.3, 10.5.1: https://learn.microsoft.com/en-us/windows/apps/publish/store-policies
2. App package requirements for MSI/EXE app: https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msi/app-package-requirements
3. App package requirements for MSIX app - including manual manifests, WACK, and Store re-signing: https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/app-package-requirements
4. Shared app capability declarations - `runFullTrust`, `allowElevation`, restricted-capability approval: https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/app-capability-declarations
5. Prepare to package a desktop application - compatibility/elevation caveats: https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-prepare
6. Create an app submission for your MSI/EXE app - Partner Center and listing checklist: https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msi/create-app-submission
7. Windows App Certification Kit - local validation and Desktop Bridge support: https://learn.microsoft.com/en-us/windows/uwp/debug-test-perf/windows-app-certification-kit
