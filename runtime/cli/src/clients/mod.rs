// Copyright (c) 2023 - 2025 Restate Software, Inc., Restate GmbH.
// All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

mod admin_client;
mod admin_interface;
mod errors;

pub use self::admin_client::AdminClient;
pub use self::admin_client::Error as MetasClientError;
pub use self::admin_interface::AdminClientInterface;
