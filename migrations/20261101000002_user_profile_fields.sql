-- Profile + press-card details. Text columns are utf8mb4 explicitly so Bengali is always safe.
-- photo / bio / designation / blood group: for everyone.
-- journalist_number / card dates: used for people with the Journalist role (NULL = not set).
ALTER TABLE users
    ADD COLUMN photo VARCHAR(500) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NOT NULL DEFAULT '',
    ADD COLUMN bio VARCHAR(500) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NOT NULL DEFAULT '',
    ADD COLUMN designation VARCHAR(120) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NOT NULL DEFAULT '',
    ADD COLUMN blood_group VARCHAR(5) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NOT NULL DEFAULT '',
    ADD COLUMN journalist_number VARCHAR(50) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NULL,
    ADD COLUMN card_issue_date DATE NULL,
    ADD COLUMN card_expiry_date DATE NULL,
    ADD UNIQUE KEY uq_users_journalist_number (journalist_number);
