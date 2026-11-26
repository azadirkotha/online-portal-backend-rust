-- Add migration script here
CREATE TABLE IF NOT EXISTS breaking_news (
    id INT AUTO_INCREMENT PRIMARY KEY,
    headline VARCHAR(255) NOT NULL,
    -- site path ("/news/some-slug") or a full http(s) URL
    path VARCHAR(500) NOT NULL DEFAULT '',
    -- 1 = shown in the public breaking list
    is_active TINYINT(1) NOT NULL DEFAULT 1,
    created_by INT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
    KEY idx_breaking_active (is_active, created_at),
    CONSTRAINT fk_breaking_creator FOREIGN KEY (created_by) REFERENCES users(id) ON DELETE SET NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;