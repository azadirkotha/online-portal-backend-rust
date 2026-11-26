-- One table for every navigation list on the site (see LOCATIONS in src/models/menu.rs):
-- header, footer_quick, footer_sections, footer_about.
-- An item is a category, a sub category, or a custom link. parent_id makes it a dropdown item (2 levels max).
CREATE TABLE IF NOT EXISTS menu_items (
    id INT AUTO_INCREMENT PRIMARY KEY,
    location VARCHAR(30) NOT NULL,
    parent_id INT NULL,
    kind VARCHAR(20) NOT NULL,                 -- 'category' | 'sub_category' | 'link'
    category_id INT NULL,
    sub_category_id INT NULL,
    label VARCHAR(120) NOT NULL DEFAULT '',    -- optional override for categories, required for links
    url VARCHAR(500) NOT NULL DEFAULT '',      -- links only
    open_in_new_tab BOOLEAN NOT NULL DEFAULT FALSE,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    position INT NOT NULL DEFAULT 0,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
    -- a category / sub category can appear once per menu (NULLs do not collide, so links are unlimited)
    UNIQUE KEY uq_menu_items_category (location, category_id),
    UNIQUE KEY uq_menu_items_sub_category (location, sub_category_id),
    KEY idx_menu_items_order (location, parent_id, position),
    CONSTRAINT fk_menu_items_parent FOREIGN KEY (parent_id) REFERENCES menu_items(id) ON DELETE CASCADE,
    CONSTRAINT fk_menu_items_category FOREIGN KEY (category_id) REFERENCES categories(id) ON DELETE CASCADE,
    CONSTRAINT fk_menu_items_sub_category FOREIGN KEY (sub_category_id) REFERENCES sub_categories(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
