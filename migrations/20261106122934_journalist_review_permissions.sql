-- Add migration script here
INSERT IGNORE INTO permissions (name) VALUES
    ('user:delete'),
    ('journalist:review');