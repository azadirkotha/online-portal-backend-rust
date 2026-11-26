-- =====================================================================
--  Access control: permissions, roles and the starter data
--  One consolidated migration (replaces the separate permission files).
--  Order matters: tables -> permissions -> roles -> role grants -> seed data.
--  The Super Admin role is kept in sync with every permission by seed.rs on start-up.
-- =====================================================================


-- ---------------------------------------------------------------------
-- 1. Tables
-- ---------------------------------------------------------------------

-- Every permission that exists in the system
CREATE TABLE IF NOT EXISTS permissions (
    id   INT AUTO_INCREMENT PRIMARY KEY,
    name VARCHAR(100) NOT NULL UNIQUE
);

-- Roles: named bundles of permissions
CREATE TABLE IF NOT EXISTS roles (
    id          INT AUTO_INCREMENT PRIMARY KEY,
    name        VARCHAR(100) NOT NULL UNIQUE,
    description VARCHAR(255) NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS role_permissions (
    role_id       INT NOT NULL,
    permission_id INT NOT NULL,
    PRIMARY KEY (role_id, permission_id),
    FOREIGN KEY (role_id)       REFERENCES roles(id)       ON DELETE CASCADE,
    FOREIGN KEY (permission_id) REFERENCES permissions(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS user_roles (
    user_id INT NOT NULL,
    role_id INT NOT NULL,
    PRIMARY KEY (user_id, role_id),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (role_id) REFERENCES roles(id) ON DELETE CASCADE
);

-- Permissions granted directly to one user (outside any role)
CREATE TABLE IF NOT EXISTS user_permissions (
    user_id       INT NOT NULL,
    permission_id INT NOT NULL,
    PRIMARY KEY (user_id, permission_id),
    FOREIGN KEY (user_id)       REFERENCES users(id)       ON DELETE CASCADE,
    FOREIGN KEY (permission_id) REFERENCES permissions(id) ON DELETE CASCADE
);


-- ---------------------------------------------------------------------
-- 2. Unique usernames / emails
--    If either statement fails, `users` has duplicates: clean them up and re-run.
--    (Run once; MySQL has no "ADD UNIQUE KEY IF NOT EXISTS".)
-- ---------------------------------------------------------------------
ALTER TABLE users ADD UNIQUE KEY uq_users_username (username);
ALTER TABLE users ADD UNIQUE KEY uq_users_email    (email);


-- ---------------------------------------------------------------------
-- 3. Permissions
-- ---------------------------------------------------------------------
INSERT IGNORE INTO permissions (name) VALUES
    -- dashboard access
    ('admin:access'),

    -- users and roles
    ('user:read'),
    ('user:create'),
    ('user:update'),
    ('user:delete'),
    ('role:read'),
    ('role:write'),
    ('role:assign'),

    -- journalists (review applications, create / edit journalists)
    ('journalist:review'),

    -- articles
    ('article:write'),        -- create / edit own drafts, upload images, submit for review
    ('article:review'),       -- see ALL articles, edit any, publish, request changes, unpublish
    ('article:delete'),       -- delete any article
    ('frontpage:manage'),     -- arrange the front page, home categories, category leads
    ('breaking:manage'),      -- breaking news ticker

    -- taxonomy
    ('category:read'),    ('category:write'),    ('category:delete'),
    ('subcategory:read'), ('subcategory:write'), ('subcategory:delete'),
    ('tag:read'),         ('tag:write'),         ('tag:delete'),

    -- site
    ('menu:read'),        ('menu:write'),
    ('settings:read'),    ('settings:write');


-- ---------------------------------------------------------------------
-- 4. Starter roles
-- ---------------------------------------------------------------------
INSERT IGNORE INTO roles (name, description) VALUES
    ('Journalist', 'Writes articles and submits them for review. Cannot publish.'),
    ('Sub Editor', 'Reviews and publishes articles and arranges the front page.');

-- Journalist
INSERT IGNORE INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id
  FROM roles r
  JOIN permissions p ON p.name IN ('admin:access', 'article:write')
 WHERE r.name = 'Journalist';

-- Sub Editor
INSERT IGNORE INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id
  FROM roles r
  JOIN permissions p ON p.name IN (
        'admin:access',
        'article:write', 'article:review', 'article:delete',
        'frontpage:manage', 'breaking:manage',
        'category:read', 'subcategory:read', 'tag:read')
 WHERE r.name = 'Sub Editor';

-- Breaking news for an existing "Admin" role, if you have one (Super Admin is handled by seed.rs).
INSERT IGNORE INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id
  FROM roles r
  JOIN permissions p ON p.name = 'breaking:manage'
 WHERE r.name = 'Admin';


-- ---------------------------------------------------------------------
-- 5. Starter header menu (only when the menu is still empty)
--    Every active category in its sort order, with its active sub categories as the dropdown.
--    Reorder / remove / add from Menus in the dashboard.
--    Needs the `categories`, `sub_categories` and `menu_items` tables to exist already.
-- ---------------------------------------------------------------------
INSERT INTO menu_items (location, parent_id, kind, category_id, position)
SELECT 'header', NULL, 'category', c.id,
       ROW_NUMBER() OVER (ORDER BY c.sort_order, c.name) - 1
  FROM categories c
 WHERE c.is_active = 1
   AND NOT EXISTS (SELECT 1 FROM menu_items WHERE location = 'header');

INSERT INTO menu_items (location, parent_id, kind, sub_category_id, position)
SELECT 'header', p.id, 'sub_category', s.id,
       ROW_NUMBER() OVER (PARTITION BY s.category_id ORDER BY s.sort_order, s.name) - 1
  FROM sub_categories s
  JOIN menu_items p
    ON p.location = 'header' AND p.category_id = s.category_id AND p.parent_id IS NULL
 WHERE s.is_active = 1
   AND NOT EXISTS (SELECT 1 FROM menu_items WHERE location = 'header' AND parent_id IS NOT NULL);