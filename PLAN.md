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
- [ ] Clarify or implement real editing modes. The current `/source` screen is a rich editor, and the "advanced" screen is metadata-only, so there is not yet a true source-vs-visual mode split.
- [ ] Extend tag management beyond create-time assignment. Existing content cannot update tags from the editor, and the tags admin page is list-only.
- [ ] Replace placeholder admin views with real workflows where needed, especially the advanced content view and the tags/assets listing screens.
- [ ] Implement template customization UI in admin; current site settings only select an existing template.

## Follow-up testing

- [x] Replace the current end-to-end expectation that non-members can access admin pages with authorization coverage for viewer/author/editor/owner role boundaries.
- [ ] Add broader authorization coverage for viewer/author/editor/owner boundaries across edit, membership, asset, preview, and render routes.
- [ ] Add tests that verify audit events and revision history record the actual acting user for web requests.
- [x] Add coverage for published output including uploaded media when a custom upload root is configured.
