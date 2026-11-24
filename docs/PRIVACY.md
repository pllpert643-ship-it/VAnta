# VAnta privacy policy

Last updated: November 18, 2026

## Information accessed

VAnta observes the local computer to display CPU, RAM, optional NVIDIA GPU statistics, running process names, process identifiers, executable paths, process memory mappings, service names and states, and startup entry names and commands. File paths and commands can contain a Windows user name or other personal information. Access depends on Windows permissions. It does not read the contents of your documents to provide these observations.

## Local storage and retention

Resource timelines are held in memory and expire according to the configured history window. Preferences, queued workload commands, job status, a bounded decision history, and bounded workload stdout/stderr logs are stored in the application data folder shown in Settings. Workload output may contain sensitive information produced by that workload. Each retained log stream is capped at 1 MiB; files remain locally until removed. Native queue history retains up to 128 terminal entries and normally removes the logs of pruned entries. Files left by an interrupted or failed cleanup may remain locally. The application does not import the separate Python edition's history.

## Transfers and sharing

The default resource observation feature does not send observations to a VAnta server. This build has no VAnta account, analytics uploader, advertising integration, or automatic crash-report upload. Windows, the Microsoft Store, graphics drivers, and external programs have their own privacy practices. Custom executables you explicitly queue run with your user permissions and may read files, access the network, or produce sensitive logs. VAnta does not sandbox those programs. Only run programs you trust.

## Exports and your choices

Snapshots, process CSV files, and diagnostic or inventory exports are created only when you request them and are written to the location you choose. They can include process names, executable paths, startup commands and local system information. Review exports before sharing them, and avoid saving them to a shared or cloud-synchronized folder unless intended. Pausing observation stops new captures after an in-flight query finishes. Reduce the history window to retain less in-memory history. Close VAnta before deleting its application data folder to remove its preferences, queue history and logs. You can remove exported files separately from the folders you chose. Uninstalling may leave explicitly exported files and, depending on Windows package data handling, local files; check those locations if you want them removed.

## Permissions and protection

VAnta uses Windows access controls on your local user profile. It does not add its own encryption to local storage or exports. The Store edition does not request administrator elevation or perform service control, startup approval writes, or system cache/standby purges. Available per-process actions require your confirmation and may be denied by Windows. Neither observation nor memory trimming guarantees improved performance.

## Contact and changes

For privacy questions, use the publisher support contact on VAnta's Microsoft Store listing. The publisher must provide that contact and a public URL for this policy before release. Future versions that change data handling should update this policy before publication.
