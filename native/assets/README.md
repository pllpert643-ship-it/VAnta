# Vanta Native branding

The original `vanta-logo.png` was supplied and selected by the project owner on
2026-11-24. It is retained unchanged. No third-party license claim is inferred.

Run `scripts/generate-store-assets.ps1` from the repository root to produce:

- `vanta-logo-512.png`: application sidebar and repository README.
- `vanta-icon-256.png`: native window/taskbar icon.
- `vanta.ico`: Windows executable icon with 16, 24, 32, 48, 64, 128 and 256 px images.
- `packaging/store/Assets/*.png`: existing Store manifest logo sizes.

Derived files preserve the complete composition and aspect ratio, use transparent
padding when required, and are resized with high-quality bicubic interpolation.
