-- Permissions for the new dashboard sections.
-- Super Admin picks them up automatically on the next start (see seed.rs).
INSERT IGNORE INTO permissions (name) VALUES
    ('category:read'),    ('category:write'),    ('category:delete'),
    ('subcategory:read'), ('subcategory:write'), ('subcategory:delete'),
    ('tag:read'),         ('tag:write'),         ('tag:delete');
