INSERT IGNORE INTO permissions (name) VALUES
    ('article:write'),      -- create/edit own drafts, upload images, submit for review
    ('article:review'),     -- see ALL articles, edit any, publish, request changes, unpublish
    ('article:delete'),     -- delete any article
    ('frontpage:manage');   -- arrange the front page, choose home categories and category leads

INSERT IGNORE INTO roles (name, description) VALUES
    ('Journalist', 'Writes articles and submits them for review. Cannot publish.'),
    ('Sub Editor', 'Reviews and publishes articles and arranges the front page.');

INSERT IGNORE INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r JOIN permissions p
  ON p.name IN ('admin:access', 'article:write')
WHERE r.name = 'Journalist';

INSERT IGNORE INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r JOIN permissions p
  ON p.name IN ('admin:access', 'article:write', 'article:review', 'article:delete', 'frontpage:manage',
                'category:read', 'subcategory:read', 'tag:read')
WHERE r.name = 'Sub Editor';
