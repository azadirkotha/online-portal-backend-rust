INSERT IGNORE INTO permissions (name) VALUES ('menu:read'), ('menu:write');

-- Starter main menu so the site has navigation immediately:
-- every active category, in its sort order, with its active sub categories as the dropdown.
-- Reorder / remove / add from Menus in the dashboard.
INSERT INTO menu_items (location, parent_id, kind, category_id, position)
SELECT 'header', NULL, 'category', c.id, ROW_NUMBER() OVER (ORDER BY c.sort_order, c.name) - 1
  FROM categories c
 WHERE c.is_active = 1;

INSERT INTO menu_items (location, parent_id, kind, sub_category_id, position)
SELECT 'header', p.id, 'sub_category', s.id, ROW_NUMBER() OVER (PARTITION BY s.category_id ORDER BY s.sort_order, s.name) - 1
  FROM sub_categories s
  JOIN menu_items p ON p.location = 'header' AND p.category_id = s.category_id AND p.parent_id IS NULL
 WHERE s.is_active = 1;
