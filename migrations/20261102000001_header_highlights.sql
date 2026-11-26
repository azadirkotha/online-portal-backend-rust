-- The (up to) three stories shown in the header bar of every page, chosen by editors.
-- Independent of the main front-page board, so a story can be in both.
CREATE TABLE IF NOT EXISTS header_highlights (
    position INT NOT NULL PRIMARY KEY,
    article_id INT NOT NULL,
    UNIQUE KEY uq_header_highlights_article (article_id),
    CONSTRAINT fk_header_highlights_article FOREIGN KEY (article_id) REFERENCES articles(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
