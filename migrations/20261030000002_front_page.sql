-- Which categories get their own section on the front page, and in what order
ALTER TABLE categories
    ADD COLUMN show_on_home BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN home_order INT NOT NULL DEFAULT 0;

-- Main section of the front page (lead / centre grid / left column).
-- An article can only be placed once (UNIQUE article_id).
CREATE TABLE IF NOT EXISTS front_page_slots (
    id INT AUTO_INCREMENT PRIMARY KEY,
    zone VARCHAR(20) NOT NULL,          -- 'lead' | 'center' | 'left'
    position INT NOT NULL,
    article_id INT NOT NULL,
    UNIQUE KEY uq_front_page_slots_article (article_id),
    KEY idx_front_page_slots_zone (zone, position),
    CONSTRAINT fk_fps_article FOREIGN KEY (article_id) REFERENCES articles(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Hand-picked lead stories per category: position 1 leads the front-page section,
-- positions 1 and 2 are the two leads on the category page.
CREATE TABLE IF NOT EXISTS category_leads (
    category_id INT NOT NULL,
    position TINYINT NOT NULL,
    article_id INT NOT NULL,
    PRIMARY KEY (category_id, position),
    KEY idx_category_leads_article (article_id),
    CONSTRAINT fk_cl_category FOREIGN KEY (category_id) REFERENCES categories(id) ON DELETE CASCADE,
    CONSTRAINT fk_cl_article FOREIGN KEY (article_id) REFERENCES articles(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
