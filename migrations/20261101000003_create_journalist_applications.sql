-- Add migration script here
CREATE TABLE journalist_applications (
  id INT AUTO_INCREMENT PRIMARY KEY,
  user_id INT NOT NULL,
  full_name VARCHAR(100) NOT NULL,
  nid_number VARCHAR(20) NOT NULL,
  phone VARCHAR(40) NOT NULL,
  occupation VARCHAR(100) NOT NULL,
  current_platform VARCHAR(150) NOT NULL DEFAULT '',
  platform_phone VARCHAR(40) NOT NULL DEFAULT '',
  about TEXT NOT NULL,
  photo_file VARCHAR(80) NOT NULL,
  nid_file VARCHAR(80) NOT NULL,
  resume_file VARCHAR(80) NOT NULL,
  status VARCHAR(20) NOT NULL DEFAULT 'pending',
  review_note VARCHAR(500) NOT NULL DEFAULT '',
  reviewed_by INT NULL,
  reviewed_at DATETIME NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE KEY uq_application_user (user_id),
  KEY idx_application_status (status),
  KEY idx_application_nid (nid_number),
  CONSTRAINT fk_application_user FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);