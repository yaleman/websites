# Plan

Reviewed against the current implementation on 2026-03-08.

## Implemented

- [x] Admin memberships UI exists for listing, adding, updating, and removing site memberships.
- [x] Rich-text editing is in place via TipTap for content creation and content editing.
- [x] Content creation supports tag selection.
- [x] Image insertion is implemented through the asset library modal.
- [x] Editor preview exists both inline in the editor and as a rendered site-template preview route.
- [x] Site settings can update the site title and selected template.
- [x] Admin frontend assets are built with TypeScript and Rspack.

## Remaining work

- [x] first user to log into a system gets created as an admin user
- [x] Clarify or implement real editing modes. The rich editor now lives at `/edit`, and markdown editing is an explicit in-editor mode instead of a separate misleading route.
- [x] Extend tag management beyond create-time assignment. Existing content can now update tags from the editor, and the tags admin page supports create/delete workflows.
- [x] Replace placeholder admin views with real workflows where needed, especially the advanced content view and the assets listing screens.
- [x] Implement template customization UI in admin; site settings now support per-site template overrides.
- [x] add an option to provide an image URL for import instead of just uploading

## Follow-up testing

- [x] Replace the current end-to-end expectation that non-members can access admin pages with authorization coverage for viewer/author/editor/owner role boundaries.
- [x] Add broader authorization coverage for viewer/author/editor/owner boundaries across edit, membership, asset, preview, and render routes.
- [x] Add tests that verify audit events and revision history record the actual acting user for web requests.
- [x] Add coverage for published output including uploaded media when a custom upload root is configured.
