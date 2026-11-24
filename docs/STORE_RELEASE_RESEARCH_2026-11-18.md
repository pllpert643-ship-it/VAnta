# VAnta Store MSIX release research — 2026-11-18

Scope: bounded verification of Microsoft requirements for an existing Rust Win32 application. Read `docs/MICROSOFT_STORE_READINESS.md`; implementation findings in that document remain parent-supplied evidence. This research did not inspect application code, build packages, access Partner Center, or test Windows behavior. No further agents were spawned, following the explicit task instruction over the research skill's delegation default.

Official Microsoft sources were checked on **November 18, 2026**. The policy page currently identifies **version 7.20**, published **September 15, 2026**, effective **October 22, 2026**. [1]

## Release direction

**Recommendation:** prepare a fully packaged, medium-integrity MSIX candidate, subject to the parent's feature audit. Exclude undocumented startup-state writes and unverified system-wide standby mutations from the Store artifact. Treat retained administrator functionality as a separate eligibility question requiring Microsoft clarification. This is an engineering recommendation, not a certification decision. Evidence and remaining unknowns follow.

## `runFullTrust` and `allowElevation`

Microsoft defines full-trust desktop applications as **medium integrity**, outside AppContainer. Their package needs the restricted capability `runFullTrust`; the name does not mean administrator privileges. [2]

`allowElevation` is a separate restricted capability for Microsoft partners or enterprise organizations maintaining existing desktop functionality dependent on auto-elevation at launch or runtime. Store approval uses strict criteria. Restricted capabilities require an explanation during submission, and sideloading success does not establish Store approval. [2]

A conflicting general preparation page says that apps needing elevation for any functionality will not be accepted. It also says packaged applications must work as the interactive user. The capability-specific page therefore establishes a reviewed exception path, but does not establish VAnta's eligibility. [2][3]

**Recommendation:** declare `runFullTrust` for the ordinary desktop executable and explain its purpose in submission notes. Omit `allowElevation` if the submitted feature set needs no elevated behavior. If elevation remains, obtain a written answer for the exact feature and launch mechanism before treating it as release-ready; a manifest declaration alone is insufficient. [2][3]

## Store signing and unsigned submission

The Store MSIX requirements explicitly say a CA-trusted signature, purchased certificate, CA `.pfx`/`.cer`, and hardware token are unnecessary. Microsoft re-signs MSIX/AppX packages after certification. The EXE/MSI installer submission route has different publisher-signing requirements. [4]

Microsoft's first-app guide goes further: for Store MSIX publication, skip the code-signing setup step. **Practical conclusion:** generate the Store upload without publisher signing; do not make acquiring a commercial signing certificate a prerequisite. This conclusion follows that explicit workflow instruction, rather than interpreting “no CA-trusted signature” alone as proof. No actual VAnta unsigned upload was attempted. [5]

Keep installation testing distinct. Microsoft's Windows 11 unsigned-install feature requires a special publisher OID and `Add-AppxPackage -AllowUnsigned`; Microsoft says it is for quick testing, not broad distribution. It changes identity relative to a signed package. **Do not append that OID to the Partner Center publisher value for a Store upload.** For representative local testing, use a separately trusted development signature or the documented unpackaged-layout registration workflow, then verify the final Store-distributed build. [6][7]

## Manual manifest and identity for Rust

Microsoft documents a command-line packaging workflow using `AppxManifest.xml`, payload files/assets, and Windows SDK `MakeAppx.exe`; Visual Studio need not compile the app. Applying that workflow to an existing Rust-built Win32 executable is an engineering inference from this compiler-independent packaging model. [7]

Use the manual template as the baseline:

- `Identity`: actual package name, publisher, four-part version, and architecture matching the payload.
- `Properties`, `Resources`, `Dependencies`, and `Applications`: display information, real logo paths, languages, and `Windows.Desktop` requirements.
- `Application`: actual relative executable path, `uap10:RuntimeBehavior="packagedClassicApp"`, and `uap10:TrustLevel="mediumIL"`.
- `Capabilities`: `<rescap:Capability Name="runFullTrust"/>` with its declared XML namespace.

The `uap10` runtime/trust attributes require Windows 10 version 2004/build 19041 or later. Microsoft's template specifies an `EntryPoint` alternative for older systems. Choose the OS floor from actual application requirements and testing. Do not copy historical template OS values as a product support decision. [7]

Reserve/select the product in Partner Center and copy its identity details. `Package/Identity/Name`, `Package/Identity/Publisher`, and `Package/Properties/PublisherDisplayName` must match the account/product values; display branding is not a substitute for identity. [8] Manifest values are case-sensitive, including punctuation and spacing. Plain `.msix` is a supported submission format; `.msixupload` is not universally mandatory. Microsoft directs submitters to run WACK. As a conservative versioning convention, use `major.minor.build.0`; the requirements page reserves the fourth component for Store use in its Windows 10/11 UWP package guidance. [4]

**Recommendation:** create a complete payload MSIX rather than accidentally copying a sparse/external-location identity-package example. Microsoft's distribution guide says the sparse package itself is not submitted through the direct Store MSIX route; its underlying installer can use the separate MSI/EXE route. [9]

## Privacy and Windows modifications

Policy **10.5.1 explicitly includes Win32 and Desktop Bridge products among types that always need privacy policies**. Provide the privacy-policy URL in Partner Center. Describe information accessed as well as collected/transmitted, purposes, storage/security, disclosures, and user controls. A local-only design does not remove this explicit Win32 requirement. [1]

Policy **10.2.8 requires both supported methods and user consent** for Windows-setting or experience changes, and identifies unsupported uses of undocumented/unsupported APIs as disallowed methods. Consent, administrator access, or successful execution alone does not resolve that requirement. [1]

### Startup control

No official Microsoft supported-write contract for `Explorer\StartupApproved` was located in this bounded investigation. This is an evidence gap, not proof that Microsoft has published no relevant material anywhere.

The documented `StartupTask` API supports packaged desktop startup registration through `windows.startupTask`. Its enumeration is scoped to the current package; enabling cannot override a task disabled by the user in Task Manager. It therefore does not establish a supported replacement for controlling arbitrary third-party startup entries. [10]

**Recommendation:** remove `StartupApproved` mutation from the Store artifact pending a supported contract; provide inspection and a handoff to Windows startup controls. If VAnta needs its own launch-at-login feature, use its package's documented startup extension/API and preserve the user's disable choice. These are product recommendations from the evidence above, not a Microsoft verdict on the current implementation. [1][10]

### Standby and memory changes

Microsoft documents `NtSetSystemInformation`, but says it supports only a subset of information classes and lacks a Windows SDK header declaration. The generic function reference does **not** document a standby-purge command contract. Searches for `SystemMemoryListInformation` / `MemoryPurgeStandbyList` did not establish such a Microsoft contract. Do not describe the entire function as undocumented merely because a particular command is unverified. [11]

Microsoft separately documents `EmptyWorkingSet` as removing pages from a specified process's working set. It requires query/query-limited-information access plus `PROCESS_SET_QUOTA`, and reports failure through its return value/GetLastError. That operation is not documentation for a global standby-list purge. [12]

**Recommendation:** exclude unverified global standby mutations. Evaluate any documented per-process operation separately against its actual target, permissions, user intent, failure behavior, and measured benefit; do not present a working-set operation as an equivalent replacement or as certification approval. [1][12]

## Exact unknowns and evidence needed

| Unknown | Evidence that resolves it |
| --- | --- |
| Which runtime paths actually need administrator privileges? | Parent audit of launch manifests, elevation calls, child processes, and privileged feature entry points; standard-user execution results. |
| Can this publisher and these retained features obtain `allowElevation` approval? | Microsoft/Partner Center response identifying the publisher, exact operations, elevation mechanism, and required capability approval. Published criteria do not decide this case. [2][3] |
| Does the exact unsigned VAnta artifact pass Store ingestion? | Upload the generated MSIX with real identity and retain validation results. This research confirms the documented signing workflow, not a successful upload. [4][5] |
| What are VAnta's real Store identity values? | Authorized Partner Center identity page. Placeholder package names/publishers are not release values. [8] |
| Which `StartupApproved` keys/bytes and standby information classes are used? | Parent code audit with exact paths, constants, buffers, privileges, and call sites. No claim is made here that VAnta uses a particular native information-class number. |
| Is any retained startup/standby mutation supported for this scenario? | A Microsoft contract for the exact operation and supported Windows builds, or written Microsoft guidance. Generic API documentation and observed registry layouts do not resolve this gap. [1][10][11] |
| What data does the current build access, persist, or transmit? | Audit process/startup inspection, logs, crash reports, network calls, retention and deletion. This research does not establish a “no collection” claim. [1] |
| Does the actual package work across its claimed systems? | Schema/MakeAppx validation, applicable WACK results, and clean-machine install, launch, upgrade and uninstall evidence; standard-user paths and failure handling. These are proposed release checks, not results obtained here. [3][4] |

The readiness document's elevation conflict remains real; the new verification adds an explicit Microsoft instruction to skip signing for Store MSIX and separates unsigned upload from Windows' special unsigned-development identity. Missing operation contracts and account-specific approval remain unresolved. No packaging or policy research can substitute for the parent's implementation audit or Microsoft's certification decision.

## Official primary sources

All accessed November 18, 2026 through direct HTTPS retrieval of Microsoft pages; browser-tool requests returned no usable content. References identify the owning Microsoft documentation and relevant sections; search-result absence is reported only as a bounded research limitation.

1. [Microsoft Store Policies](https://learn.microsoft.com/en-us/windows/apps/publish/store-policies) — version/date header; 10.2.8; 10.5.1.
2. [App capability declarations](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/app-capability-declarations) — medium IL; Restricted capabilities; Full Trust Permission Level; Elevation.
3. [Prepare to package a desktop application](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-prepare) — elevated privileges and interactive-user compatibility.
4. [App package requirements for MSIX app](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/app-package-requirements) — package types; code signing; manual manifest; WACK; version numbering.
5. [Publish your first Windows app](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/publish-first-app) — Step 2: Set up code signing, Store MSIX instruction.
6. [Create an unsigned MSIX package](https://learn.microsoft.com/en-us/windows/msix/package/unsigned-package) — Windows 11 testing, special OID, identity and installation limitations.
7. [Generating MSIX package components](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-manual-conversion) — manual template, OS compatibility, layout registration and packaging.
8. [View app identity details](https://learn.microsoft.com/en-us/windows/apps/publish/view-app-identity-details) — Partner Center identity/manifest mapping.
9. [Choose a distribution path for your Windows app](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/choose-distribution-path) — packaging with external location limitations.
10. [StartupTask class](https://learn.microsoft.com/en-us/uwp/api/windows.applicationmodel.startuptask?view=winrt-26100) — desktop startup extension, package scope, user-disabled state.
11. [NtSetSystemInformation](https://learn.microsoft.com/en-us/windows/win32/sysinfo/ntsetsysteminformation) — parameters and remarks; absence of a documented standby-purge contract in this reference.
12. [EmptyWorkingSet](https://learn.microsoft.com/en-us/windows/win32/api/psapi/nf-psapi-emptyworkingset) — process scope, required access rights, error handling.

