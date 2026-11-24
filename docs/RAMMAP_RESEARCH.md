# RAMMap static investigation and VAnta read-only subset

Date: 2026-11-16. Scope: supplied `C:/Users/MSI GF63 Thin/Downloads/RAMMap`, inspected without target execution; integration is this report and `reference/rammap-evidence/` only. Shared application changes were neither edited nor reverted. Primary skill: `reverse-engineer-anything`; independent workers were requested for official corroboration and subset feasibility. The feasibility worker supplied a saved note; the official-source worker did not complete corroboration, so the primary investigator fetched Microsoft directly.

## Exact artifact identity - observed

| Artifact | Header classification | Bytes | File/product version | SHA-256 |
| --- | --- | ---: | --- | --- |
| `Eula.txt` | text | 7,490 | n/a | `8329bcbadc7f81539a4969ca13f0be5b8eb7652b912324a1926fc9bfb6ec005a` |
| `RAMMap.exe` | PE32 / x86 | 677,792 | 1.61 | `9200237404fc7936ed5854d904907f0ab9a4c689f38f98b49d30599a5498c84d` |
| `RAMMap64a.exe` | PE32+ / ARM64 | 379,792 | 1.61 | `9d110b7af0f89899bd6c8e7b3d7e689dd1e0f5471ad7aef03e8288ba169f4de3` |

Source: `LOCAL-IDENTITY-001` (`artifact-inventory.json`), corroborated by REA PE source digests and `LOCAL-PE-DECODE-001`. Both executables report **1.61** for file and product version, description "RamMap - physical memory analyzer", company "Sysinternals - www.sysinternals.com". PowerShell Authenticode returned **Valid / Signature verified** for both, signer Microsoft Corporation, thumbprint `8740DF4ACB749640AD318E4BE842F72EC651AD80`, with Microsoft timestamp-service certificates. This is the host's verification result at capture time, not an independent certificate-chain audit. Eula.txt is plain text, not an unsigned executable; its unsupported-signature result is not a malware finding. Reading/hashing it did not accept it.

**Version discrepancy:** the official Microsoft page fetched on 2026-11-16 advertises **RAMMap v1.63, published March 26, 2026**. The supplied 1.61 bytes were not replaced, updated, or represented as current. These are downloaded artifacts at the supplied path, not proof of a registered installation. No standalone RAMMap64.exe was present in this directory inventory.

## Retained evidence and tool execution

All filenames below are relative to `reference/rammap-evidence/`. `ev_*` identifiers are exact REA IDs; `LOCAL-*` / `OFFICIAL-*` are explicitly investigator-assigned evidence labels, not REA IDs.

| Evidence | Saved source | Meaning / boundary |
| --- | --- | --- |
| `LOCAL-IDENTITY-001` | `artifact-inventory.json` | Recursive file inventory, SHA-256, versions and host Authenticode result. |
| `ev_b4085bb6d682c77d584ba398ed9e2f0807a82f6cff4e97fedffc5d1793e54183` | `RAMMap.pe-resources.json` | REA exact x86 resource tree, payload ranges/hashes, icon relationships; 22 payloads. |
| `ev_eff2a7ec10b3493c9f1d7ea0a2802b4db55b274ffd539e7bcf587ad51552b2e2` | `RAMMap64a.pe-resources.json` | REA exact ARM64 resource tree; 21 payloads. |
| `ev_02070f9c6b6db986911df8b53513a881d0932174008fe1c4d67e99c7c61b86d5` | `RAMMap.native-overview.json` | Successful REA/Ghidra 12.1.4 x86 native overview: image base 0x400000; 1,761 procedures; 822 defined strings. Not recovered source or a feature call-chain audit. |
| `LOCAL-PE-DECODE-001` | `pe-static-details.json`, generator `inspect-static-pe.py` | Independent read-only PE import parsing, bounded resource-text runs and manifest decoding anchored to REA ranges/hashes. Text extraction is not semantic menu/dialog decoding. |
| `LOCAL-STRING-SCAN-001` | `focused-string-hits.json`, generator `inspect-focused-strings.py` | Selected printable ASCII/UTF-16LE names with exact file offsets. Presence is not call execution; search is not exhaustive. |
| `OFFICIAL-MICROSOFT-001` | `microsoft-source.json`, `microsoft-rammap.html` | Official URL, retrieval timestamp, status and saved decoded-response SHA-256. Saved text is not raw HTTP bytes. |

Existing CLI: `D:/rea-npm/rea.ps1`, rea-agents **6.5.0**. Captured `rea --help` and exact `--schema` outputs (`rea-help.txt`, `schema-*.txt`). Ran `rea inspect-pe-resources <each executable> --json`, `rea capabilities --json`, and `rea inspect <RAMMap.exe> --provider ghidra --json`; canonical outputs are retained. Capability results identified existing Ghidra 12.1.4 / Java 21.0.12.1. No setup, downloads of analysis engines, or configuration repair was performed. CLI workflow owned the ephemeral native session; no persistent GUI/database session was opened by this investigation. REA documents close-time deletion of its temporary imported Program/project; cleanup was not independently audited. After the initial overview completed, no further native engine wait or function decompilation was attempted, to prioritize the bounded read-only integration. ARM64 native decompilation was not attempted; Windows Ghidra P0 advertises x86/x64 PE support, not ARM64.

## Static findings - observations versus inference

- **Observed resources:** both manifests request `requireAdministrator` and `uiAccess=false`. This is a declaration in the shipped bytes, not a UAC/runtime test. Both menus include Open, Save, Find, Refresh, Settings/Colors and five Empty actions (working sets, system working set, modified list, standby list, priority-0 standby list). Usage resources mention `.rmp`, output-to-file-and-exit, `-run32`, and `-E[wsmt0]`. No command was executed.
- **Observed embedded payload:** x86 resource `BINRES/RCRAMMAP64`, language 1033, offset **293688**, length **362896**, SHA-256 `f80852a5ab710dde6489760b3fba6907ba63a7d9704dd2fc58b387c472541bf0`, begins `MZ`. **Inference:** likely bundled x64 companion, consistent with the usage text. Its architecture/version/signature were not separately parsed/verified; it was not extracted or executed. Do not label it a driver or verified x64 executable merely from its name.
- **Observed imports:** both standalone binaries import `OpenProcess`, `DeviceIoControl`, `CreateProcessW`, `OpenProcessToken`, `AdjustTokenPrivileges`, `LookupPrivilegeValueW`, and service APIs including `OpenServiceW`, `StartServiceW`, `ControlService`, `QueryServiceStatusEx`. Complete DLL/name lists are preserved in `pe-static-details.json`. **Inference only:** these enable candidate process/device/privilege/service paths; imports alone do not establish which path RAMMap uses or prove a driver is installed/loaded.
- **Observed selected strings:** canonical seven tab labels are present, together with `NtQuerySystemInformation`, `NtSetSystemInformation`, `\Device\FileInfo`, `SeProfileSingleProcessPrivilege`, `SeDebugPrivilege`, `SeIncreaseQuotaPrivilege`. Outer x86 examples: `NtQuerySystemInformation` at **0x373e0**, `NtSetSystemInformation` at **0x37d7c**, `Processes` at **0x38648**; ARM64 `Use Counts` at **0x446d8**. Some x86 scan hits lie inside the embedded resource: they are not independent outer-module code evidence. The x86 `AUse Counts` run at 0x3860a includes an adjacent printable byte; do not interpret that prefix as the actual UI label. **Unknown:** dynamic resolutions, information classes, structures, PFN algorithms and actual call sites; no call-chain/pseudocode analysis establishes them.
- **Observed provider limit:** Ghidra reports a `RamMap.pdb` reference (GUID `a1a0aa6f-b428-491b-af8d-f859a501895d`, age 1), load state unknown. Original names/types and behavior remain unverified. Resource and defined-string inventories are not proof of enabled/reachable UI or runtime correctness.

## Canonical features and honest VAnta coverage

Official Microsoft source: https://learn.microsoft.com/en-us/sysinternals/downloads/rammap (retrieved 2026-11-16). Descriptions below are paraphrased from that page; resource/string presence corroborates labels, not implementation internals. The current VAnta scope is user-reported on 2026-11-16: read-only **Overview / Processes / Process mappings**, JSON snapshots, backend being implemented with **psutil + GetPerformanceInfo**. This report does not certify those concurrent changes or test their completion.

| Canonical RAMMap capability | VAnta coverage / explicit missing behavior |
| --- | --- |
| Use Counts - physical usage by type and paging list | **Not implemented equivalently.** Overview aggregates can report total/available/used, commit/cache/pools where available; no RAMMap page-use/page-list matrix or physical-memory reconciliation. |
| Processes - working-set sizes by process | **Partial conceptual overlap.** Process RSS/working-set and available counters; no page ownership, shared-page attribution, or identical counter semantics promised. Private bytes are not private working-set bytes; process working sets must not be summed into system used RAM. |
| Priority Summary - standby sizes by priority | **Unimplemented.** No priority-tier standby accounting. |
| Physical Pages - usage of individual physical pages | **Unimplemented.** No PFN enumeration or per-physical-page use/owner/state. |
| Physical Ranges - physical memory address ranges | **Unimplemented.** Process virtual mappings are not physical ranges. |
| File Summary - resident file data attributed by file | **Unimplemented.** Aggregate system cache and process mapped-file names do not establish globally resident cached bytes by file. |
| File Details - physical pages associated with a file | **Unimplemented.** Process mappings are virtual regions; no file-to-physical-page enumeration. |
| Refresh; save/load memory snapshots | **Partial conceptual overlap.** Refresh and VAnta JSON snapshots can be independent read-only features; RAMMap `.rmp` interoperability, its full physical snapshot contents, and equivalent loading are not implemented/verified. |
| Empty actions (resource-observed, not asserted by official page) | **Intentionally excluded.** No trimming, cache purge, standby/modified-list clearing, service/driver control or native setter commands. |

## Integration recommendation and unresolved checks

Keep the independently implemented, current-token read-only subset: system aggregates; per-process memory; process **virtual** mappings; explicit refresh; VAnta-owned JSON schema. Use documented collection paths available through psutil/GetPerformanceInfo, not reconstructed RAMMap algorithms or undocumented PFN/native query/setter paths. Do not copy proprietary code/artwork, ship RAMMap binaries, require administrator access, install drivers or claim full parity. Source bytes remain in Downloads; evidence stores metadata and small focused text excerpts, not a redistributed executable.

Represent denied/exited/unavailable observations explicitly (null plus reason), track capture timestamps and stale state, normalize bytes once, preserve PID-plus-creation-time identity, and avoid resampling during snapshot export. Mapping visibility varies by platform/token/process and must not silently become an empty-success result. See `reference/rammap-evidence/subset-feasibility.md` for the independent worker's source-grounded proposal; its line anchors are working-tree observations, not stable release guarantees. No app code or tests were changed/run for this documentation task.

Checklist: identity/version/hash/signature **answered**; portable PE resource/import/selected-string evidence **answered**; existing native provider/overview **answered, bounded**; function-level algorithms/runtime/driver dependence **unresolved by design**; official feature/version corroboration **answered**; integration feasibility and missing canonical features **documented, implementation not certified**. No RAMMap launch (elevated or otherwise), EULA acceptance, RAM clearing, driver installation, privilege enablement or system/REA configuration changes occurred.

