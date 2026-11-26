-- Add migration script here
INSERT IGNORE INTO permissions (name) VALUES ('breaking:manage');

-- Grant it to the roles that should manage breaking news.
-- Change the role names to match your `roles` table.
INSERT IGNORE INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id
  FROM roles r
  JOIN permissions p ON p.name = 'breaking:manage'
 WHERE r.name IN ('admin', 'sub editor');