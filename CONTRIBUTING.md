# Contributing to Claria

We welcome contributions to Claria! Before you begin, please read through this guide.

## Getting started

1. Fork the repository
2. Create a feature branch from `main`
3. Make your changes
4. Run checks locally:
   ```bash
   cargo clippy -- -D warnings
   cargo test
   cd claria-desktop-frontend && npm run lint
   ```
5. Open a pull request against `main`

## Code style

- Follow existing patterns in the codebase
- No `unwrap()` outside of tests
- Library crates accept `&SdkConfig` — they never build their own AWS configs
- Library crates never touch the filesystem (except `claria-search`)
- All `pub` types derive `Serialize` + `Deserialize`

## Questions?

Open an issue if you have questions or want to discuss a change before starting work.

---

# Contributor License Agreement

Thank you for your interest in contributing to Claria, owned and maintained by Claria AI ("We" or "Us").

This Contributor License Agreement ("Agreement") documents the rights granted by contributors to Us. This is a legally binding document, so please read it carefully before agreeing to it.

**All contributors must agree to this CLA before their pull request can be merged.** When you open your first pull request, a bot will comment asking you to sign. Simply reply with:

> I have read the CLA and I agree to it.

The bot records your agreement and all your future PRs will pass the CLA check automatically.

### 1. Definitions

"You" means the individual copyright owner who submits a Contribution to Us.

"Contribution" means any original work of authorship, including any modifications or additions to an existing work, that You intentionally submit to Us for inclusion in Claria. "Submit" means any form of electronic or written communication sent to Us, including but not limited to pull requests, patches, commits, issues, and comments on any of these.

### 2. Copyright Assignment

You hereby irrevocably assign to Us all right, title, and interest worldwide in and to the copyright in Your Contributions. This includes the right to sublicense, relicense, and distribute Your Contributions under any license, including proprietary licenses.

To the extent that the above assignment is ineffective under applicable law, You grant to Us a perpetual, irrevocable, non-exclusive, worldwide, royalty-free, unrestricted license to use, reproduce, prepare derivative works of, publicly display, publicly perform, sublicense, and distribute Your Contributions and any derivative works thereof, under any license and for any purpose.

### 3. Patent License

You grant to Us a perpetual, irrevocable, non-exclusive, worldwide, royalty-free patent license to make, have made, use, offer to sell, sell, import, and otherwise transfer Your Contributions, where such license applies only to those patent claims licensable by You that are necessarily infringed by Your Contributions alone or by combination of Your Contributions with the work to which such Contributions were submitted.

### 4. Rights and Representations

You represent that:

(a) You are legally entitled to grant the above assignment and license. If Your employer has rights to intellectual property that You create, You represent that You have received permission to make Contributions on behalf of that employer, or that Your employer has waived such rights for Your Contributions to Claria.

(b) Each of Your Contributions is Your original creation. You represent that Your Contributions include complete details of any third-party license or other restriction of which You are aware and which is associated with any part of Your Contributions.

### 5. Moral Rights

To the fullest extent permitted under applicable law, You waive and agree not to assert any moral rights You may have in Your Contributions.

### 6. Distribution

We agree that all Contributions will be distributed under the GNU General Public License, version 3 (GPL-3.0-only), or a compatible open-source license. Our right to sublicense and relicense (as granted in Section 2) allows Us to also offer the software under additional license terms, such as a commercial license, alongside the open-source release.

### 7. No Obligation

You understand that We are not obligated to use or include Your Contributions in any project and that the decision to use or include Your Contributions is at Our sole discretion.

### 8. Agreement

By commenting "I have read the CLA and I agree to it." on a pull request, You accept and agree to the terms and conditions of this Agreement for Your present and future Contributions submitted to Claria.
