CREATE TABLE IF NOT EXISTS articles (
    id INT AUTO_INCREMENT PRIMARY KEY,
    title VARCHAR(255) NOT NULL,
    slug VARCHAR(255) NOT NULL DEFAULT '',
    summary VARCHAR(500) NOT NULL DEFAULT '',
    body MEDIUMTEXT NOT NULL,
    -- news | opinion | editorial (drives the "popular / editorial / opinion" tabs)
    kind VARCHAR(20) NOT NULL DEFAULT 'news',
    news_source VARCHAR(255) NOT NULL DEFAULT '',
    featured_image VARCHAR(500) NOT NULL DEFAULT '',
    featured_image_caption VARCHAR(500) NOT NULL DEFAULT '',
    -- draft -> pending -> published   (or pending -> changes_requested -> pending ...)
    status VARCHAR(20) NOT NULL DEFAULT 'draft',
    review_note VARCHAR(1000) NOT NULL DEFAULT '',
    author_id INT NOT NULL,
    reviewed_by INT NULL,
    submitted_at DATETIME NULL,
    reviewed_at DATETIME NULL,
    published_at DATETIME NULL,
    view_count INT NOT NULL DEFAULT 0,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
    KEY idx_articles_status_published (status, published_at),
    KEY idx_articles_kind (kind, status, published_at),
    KEY idx_articles_author (author_id),
    CONSTRAINT fk_articles_author FOREIGN KEY (author_id) REFERENCES users(id) ON DELETE RESTRICT,
    CONSTRAINT fk_articles_reviewer FOREIGN KEY (reviewed_by) REFERENCES users(id) ON DELETE SET NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS article_categories (
    article_id INT NOT NULL,
    category_id INT NOT NULL,
    PRIMARY KEY (article_id, category_id),
    KEY idx_article_categories_category (category_id),
    CONSTRAINT fk_ac_article FOREIGN KEY (article_id) REFERENCES articles(id) ON DELETE CASCADE,
    -- a category that still has articles cannot be deleted by accident
    CONSTRAINT fk_ac_category FOREIGN KEY (category_id) REFERENCES categories(id) ON DELETE RESTRICT
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS article_sub_categories (
    article_id INT NOT NULL,
    sub_category_id INT NOT NULL,
    PRIMARY KEY (article_id, sub_category_id),
    KEY idx_article_sub_categories_sub (sub_category_id),
    CONSTRAINT fk_asc_article FOREIGN KEY (article_id) REFERENCES articles(id) ON DELETE CASCADE,
    CONSTRAINT fk_asc_sub FOREIGN KEY (sub_category_id) REFERENCES sub_categories(id) ON DELETE RESTRICT
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS article_tags (
    article_id INT NOT NULL,
    tag_id INT NOT NULL,
    PRIMARY KEY (article_id, tag_id),
    KEY idx_article_tags_tag (tag_id),
    CONSTRAINT fk_at_article FOREIGN KEY (article_id) REFERENCES articles(id) ON DELETE CASCADE,
    CONSTRAINT fk_at_tag FOREIGN KEY (tag_id) REFERENCES tags(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;